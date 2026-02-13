#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct JpegRect {
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
}

type JpegReadFn = unsafe fn(*mut c_void, *mut u8, usize) -> usize;
type JpegSkipFn = unsafe fn(*mut c_void, usize) -> usize;

#[derive(Clone, Copy)]
struct JpegIoDispatch {
    user: *mut c_void,
    read: JpegReadFn,
    skip: JpegSkipFn,
}

struct JpegThumbContext {
    out_bits: *mut u8,
    out_len: usize,
    thumb_width: usize,
    thumb_height: usize,
    source_width: usize,
    source_height: usize,
    max_right: usize,
    max_bottom: usize,
    pixels_written: usize,
}

impl JpegThumbContext {
    fn set_thumb_pixel(&mut self, x: usize, y: usize, on: bool) {
        if x >= self.thumb_width || y >= self.thumb_height {
            return;
        }
        if self.out_bits.is_null() || self.out_len == 0 {
            return;
        }
        // SAFETY: Pointer and length are initialized from a valid mutable slice
        // in decode_jpeg_thumbnail_stream and stay valid for callback lifetime.
        let out = unsafe { core::slice::from_raw_parts_mut(self.out_bits, self.out_len) };
        mono_set_pixel(out, self.thumb_width, x, y, on);
    }
}

static mut JPEG_IO_DISPATCH: Option<JpegIoDispatch> = None;
static mut JPEG_THUMB_CTX: *mut JpegThumbContext = core::ptr::null_mut();

#[inline]
unsafe fn jpeg_io_dispatch_load() -> Option<JpegIoDispatch> {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::read(core::ptr::addr_of!(JPEG_IO_DISPATCH)) }
}

#[inline]
unsafe fn jpeg_io_dispatch_store(value: Option<JpegIoDispatch>) {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(JPEG_IO_DISPATCH), value) }
}

#[inline]
unsafe fn jpeg_thumb_ctx_load() -> *mut JpegThumbContext {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::read(core::ptr::addr_of!(JPEG_THUMB_CTX)) }
}

#[inline]
unsafe fn jpeg_thumb_ctx_store(value: *mut JpegThumbContext) {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(JPEG_THUMB_CTX), value) }
}

struct JpegCallbackGuard;

impl JpegCallbackGuard {
    unsafe fn install(io: JpegIoDispatch, thumb_ctx: *mut JpegThumbContext) -> Option<Self> {
        // SAFETY: Access to global callback state is guarded by this install/drop pair;
        // only one JPEG decode is allowed concurrently.
        unsafe {
            if jpeg_io_dispatch_load().is_some() || !jpeg_thumb_ctx_load().is_null() {
                return None;
            }
            jpeg_io_dispatch_store(Some(io));
            jpeg_thumb_ctx_store(thumb_ctx);
        }
        Some(Self)
    }
}

impl Drop for JpegCallbackGuard {
    fn drop(&mut self) {
        // SAFETY: Clear global callback context on scope exit.
        unsafe {
            jpeg_io_dispatch_store(None);
            jpeg_thumb_ctx_store(core::ptr::null_mut());
        }
    }
}

unsafe extern "C" {
    fn jd_prepare(
        jd: *mut c_void,
        infunc: Option<unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> u32>,
        pool: *mut c_void,
        sz_pool: u32,
        device: *mut c_void,
    ) -> i32;

    fn jd_decomp(
        jd: *mut c_void,
        outfunc: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut JpegRect) -> u32>,
        scale: u8,
    ) -> i32;
}

const JDR_OK: i32 = 0;
const JDR_INTR: i32 = 1;
const JDR_INP: i32 = 2;
const JDR_MEM1: i32 = 3;
const JDR_MEM2: i32 = 4;
const JDR_PAR: i32 = 5;
const JDR_FMT1: i32 = 6;
const JDR_FMT2: i32 = 7;
const JDR_FMT3: i32 = 8;

fn jpeg_jdr_name(status: i32) -> &'static str {
    match status {
        JDR_OK => "ok",
        JDR_INTR => "intr",
        JDR_INP => "inp",
        JDR_MEM1 => "mem1",
        JDR_MEM2 => "mem2",
        JDR_PAR => "par",
        JDR_FMT1 => "fmt1",
        JDR_FMT2 => "fmt2",
        JDR_FMT3 => "fmt3_progressive_or_unsupported",
        _ => "unknown",
    }
}

unsafe extern "C" fn jpeg_in_callback(_jd: *mut c_void, buff: *mut u8, nbyte: u32) -> u32 {
    let want = nbyte as usize;
    if want == 0 {
        return 0;
    }

    // SAFETY: Read-only copy from global dispatch installed by JpegCallbackGuard.
    // SAFETY: Read callback dispatch installed by JpegCallbackGuard.
    let dispatch = unsafe { jpeg_io_dispatch_load() };
    let Some(io) = dispatch else {
        return 0;
    };

    let read = if buff.is_null() {
        // SAFETY: Function pointer/user context are installed for this decode.
        unsafe { (io.skip)(io.user, want) }
    } else {
        // SAFETY: Function pointer/user context are installed for this decode.
        unsafe { (io.read)(io.user, buff, want) }
    };
    read.min(want) as u32
}

unsafe extern "C" fn jpeg_out_callback(
    _jd: *mut c_void,
    bitmap: *mut c_void,
    rect: *mut JpegRect,
) -> u32 {
    if bitmap.is_null() || rect.is_null() {
        return 0;
    }

    // SAFETY: Global pointer is installed for this decode and cleared by guard.
    // SAFETY: Read callback output context installed by JpegCallbackGuard.
    let ctx_ptr = unsafe { jpeg_thumb_ctx_load() };
    if ctx_ptr.is_null() {
        return 0;
    }
    // SAFETY: Pointer validity is guaranteed by decode_jpeg_thumbnail_stream scope.
    let ctx = unsafe { &mut *ctx_ptr };

    // SAFETY: Decoder provides a valid rectangle pointer for callback duration.
    let rect = unsafe { *rect };
    let left = rect.left as usize;
    let top = rect.top as usize;
    let right = rect.right as usize;
    let bottom = rect.bottom as usize;
    if right < left || bottom < top {
        return 0;
    }

    let block_w = right.saturating_sub(left).saturating_add(1);
    let block_h = bottom.saturating_sub(top).saturating_add(1);
    if block_w == 0 || block_h == 0 {
        return 1;
    }

    // ESP32-S3 ROM TJPGD uses RGB888 output blocks.
    let block_pixels = block_w.saturating_mul(block_h);
    let block_bytes = block_pixels.saturating_mul(3);
    if block_bytes == 0 {
        return 0;
    }

    // SAFETY: TJPGD callback hands contiguous block bitmap bytes.
    let pixels = unsafe { core::slice::from_raw_parts(bitmap as *const u8, block_bytes) };
    let src_w = ctx.source_width.max(1);
    let src_h = ctx.source_height.max(1);

    for by in 0..block_h {
        let sy = top.saturating_add(by);
        if sy >= src_h {
            continue;
        }
        let dy = sy.saturating_mul(ctx.thumb_height) / src_h;
        if dy >= ctx.thumb_height {
            continue;
        }

        for bx in 0..block_w {
            let sx = left.saturating_add(bx);
            if sx >= src_w {
                continue;
            }
            let dx = sx.saturating_mul(ctx.thumb_width) / src_w;
            if dx >= ctx.thumb_width {
                continue;
            }

            let base = (by.saturating_mul(block_w).saturating_add(bx)).saturating_mul(3);
            if base + 2 >= pixels.len() {
                return 0;
            }

            let r = pixels[base] as u32;
            let g = pixels[base + 1] as u32;
            let b = pixels[base + 2] as u32;
            let luma = (r * 30 + g * 59 + b * 11) / 100;
            ctx.set_thumb_pixel(dx, dy, luma < 160);
        }
    }

    ctx.max_right = ctx.max_right.max(right);
    ctx.max_bottom = ctx.max_bottom.max(bottom);
    ctx.pixels_written = ctx.pixels_written.saturating_add(block_pixels);
    1
}

struct ZipJpegStream<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
> where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file: *mut embedded_sdmmc::File<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: u32,
    stream_buf: [u8; JPEG_STREAM_BYTES],
    stream_len: usize,
    stream_pos: usize,
    stream_end: bool,
    io_failed: bool,
}

impl<'a, D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>
    ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    fn new(
        file: &mut embedded_sdmmc::File<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
        entry: ZipEntryRef,
    ) -> Self {
        Self {
            file,
            entry,
            stream_offset: 0,
            stream_buf: [0u8; JPEG_STREAM_BYTES],
            stream_len: 0,
            stream_pos: 0,
            stream_end: false,
            io_failed: false,
        }
    }

    fn read_into(&mut self, out: &mut [u8]) -> Result<usize, embedded_sdmmc::Error<D::Error>> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut copied = 0usize;
        let mut no_progress_reads = 0u8;
        while copied < out.len() {
            if self.stream_pos >= self.stream_len {
                if self.stream_end {
                    break;
                }

                // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
                let file = unsafe { &mut *self.file };
                let (read_now, end_now) = read_zip_entry_chunk(
                    file,
                    self.entry,
                    self.stream_offset,
                    &mut self.stream_buf,
                )?;
                self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
                self.stream_len = read_now;
                self.stream_pos = 0;
                self.stream_end = end_now;

                if read_now == 0 {
                    no_progress_reads = no_progress_reads.saturating_add(1);
                    if self.stream_end || no_progress_reads >= 4 {
                        break;
                    }
                    continue;
                }
                no_progress_reads = 0;
            }

            let avail = (self.stream_len - self.stream_pos).min(out.len() - copied);
            out[copied..copied + avail]
                .copy_from_slice(&self.stream_buf[self.stream_pos..self.stream_pos + avail]);
            self.stream_pos = self.stream_pos.saturating_add(avail);
            copied = copied.saturating_add(avail);
        }

        Ok(copied)
    }

    fn skip_bytes(&mut self, mut len: usize) -> Result<usize, embedded_sdmmc::Error<D::Error>> {
        let mut skipped = 0usize;
        let mut no_progress_reads = 0u8;
        while len > 0 {
            if self.stream_pos >= self.stream_len {
                if self.stream_end {
                    break;
                }

                // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
                let file = unsafe { &mut *self.file };
                let (read_now, end_now) = read_zip_entry_chunk(
                    file,
                    self.entry,
                    self.stream_offset,
                    &mut self.stream_buf,
                )?;
                self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
                self.stream_len = read_now;
                self.stream_pos = 0;
                self.stream_end = end_now;

                if read_now == 0 {
                    no_progress_reads = no_progress_reads.saturating_add(1);
                    if self.stream_end || no_progress_reads >= 4 {
                        break;
                    }
                    continue;
                }
                no_progress_reads = 0;
            }

            let avail = (self.stream_len - self.stream_pos).min(len);
            self.stream_pos = self.stream_pos.saturating_add(avail);
            len = len.saturating_sub(avail);
            skipped = skipped.saturating_add(avail);
        }

        Ok(skipped)
    }

    fn read_byte(&mut self) -> Result<Option<u8>, embedded_sdmmc::Error<D::Error>> {
        let mut no_progress_reads = 0u8;
        loop {
            if self.stream_pos < self.stream_len {
                let byte = self.stream_buf[self.stream_pos];
                self.stream_pos = self.stream_pos.saturating_add(1);
                return Ok(Some(byte));
            }

            if self.stream_end {
                return Ok(None);
            }

            // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
            let file = unsafe { &mut *self.file };
            let (read_now, end_now) =
                read_zip_entry_chunk(file, self.entry, self.stream_offset, &mut self.stream_buf)?;
            self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
            self.stream_len = read_now;
            self.stream_pos = 0;
            self.stream_end = end_now;
            if read_now == 0 {
                no_progress_reads = no_progress_reads.saturating_add(1);
                if self.stream_end || no_progress_reads >= 4 {
                    return Ok(None);
                }
                continue;
            }
            no_progress_reads = 0;
        }
    }

    fn bytes_consumed(&self) -> usize {
        (self.stream_offset as usize)
            .saturating_sub(self.stream_len.saturating_sub(self.stream_pos))
    }
}

fn is_jpeg_sof_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
    )
}

fn parse_jpeg_dimensions_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> JpegDimScanResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut stream = ZipJpegStream::new(file, entry);
    let Some(first) = stream.read_byte()? else {
        return Ok(None);
    };
    let Some(second) = stream.read_byte()? else {
        return Ok(None);
    };
    if first != 0xFF || second != 0xD8 {
        return Ok(None);
    }

    loop {
        if stream.bytes_consumed() > JPEG_DIM_SCAN_MAX_BYTES {
            info!(
                "sd: jpeg decode fail reason=dims_scan_limit scanned={}",
                stream.bytes_consumed()
            );
            return Ok(None);
        }

        // Find marker prefix.
        let mut byte = loop {
            let Some(next) = stream.read_byte()? else {
                return Ok(None);
            };
            if next == 0xFF {
                break next;
            }
        };

        // Collapse fill bytes 0xFF... then read marker code.
        while byte == 0xFF {
            let Some(next) = stream.read_byte()? else {
                return Ok(None);
            };
            byte = next;
        }
        let marker = byte;

        // Stuffed 0xFF00 is possible in entropy data; ignore.
        if marker == 0x00 {
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return Ok(None);
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        let Some(len_hi) = stream.read_byte()? else {
            return Ok(None);
        };
        let Some(len_lo) = stream.read_byte()? else {
            return Ok(None);
        };
        let seg_len = u16::from_be_bytes([len_hi, len_lo]) as usize;
        if seg_len < 2 {
            return Ok(None);
        }
        let payload_len = seg_len - 2;

        if is_jpeg_sof_marker(marker) {
            if payload_len < 5 {
                return Ok(None);
            }

            let Some(_precision) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(h_hi) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(h_lo) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(w_hi) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(w_lo) = stream.read_byte()? else {
                return Ok(None);
            };
            let height = u16::from_be_bytes([h_hi, h_lo]);
            let width = u16::from_be_bytes([w_hi, w_lo]);

            if payload_len > 5 {
                let skipped = stream.skip_bytes(payload_len - 5)?;
                if skipped < payload_len - 5 {
                    return Ok(None);
                }
            }

            if width > 0 && height > 0 {
                return Ok(Some((width, height, stream.bytes_consumed())));
            }
            return Ok(None);
        }

        let skipped = stream.skip_bytes(payload_len)?;
        if skipped < payload_len {
            return Ok(None);
        }
    }
}

unsafe fn zip_jpeg_read_dispatch<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    user: *mut c_void,
    buff: *mut u8,
    len: usize,
) -> usize
where
    D: embedded_sdmmc::BlockDevice + 'a,
    T: TimeSource + 'a,
{
    if user.is_null() || buff.is_null() || len == 0 {
        return 0;
    }

    // SAFETY: `user` is installed from a live ZipJpegStream in decode scope.
    let stream =
        unsafe { &mut *(user as *mut ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) };
    if stream.io_failed {
        return 0;
    }

    // SAFETY: `buff` comes from TJPGD callback request with `len` capacity.
    let out = unsafe { core::slice::from_raw_parts_mut(buff, len) };
    match stream.read_into(out) {
        Ok(read_now) => read_now,
        Err(_) => {
            stream.io_failed = true;
            0
        }
    }
}

unsafe fn zip_jpeg_skip_dispatch<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    user: *mut c_void,
    len: usize,
) -> usize
where
    D: embedded_sdmmc::BlockDevice + 'a,
    T: TimeSource + 'a,
{
    if user.is_null() || len == 0 {
        return 0;
    }

    // SAFETY: `user` is installed from a live ZipJpegStream in decode scope.
    let stream =
        unsafe { &mut *(user as *mut ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) };
    if stream.io_failed {
        return 0;
    }

    match stream.skip_bytes(len) {
        Ok(skipped) => skipped,
        Err(_) => {
            stream.io_failed = true;
            0
        }
    }
}

fn decode_jpeg_thumbnail_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> JpegThumbDecodeResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.saturating_mul(th);
    if dst_bytes == 0 || dst_bytes > out_bits.len() {
        return Ok(None);
    }
    out_bits[..dst_bytes].fill(0);

    let Some((src_w, src_h, scanned_bytes)) = parse_jpeg_dimensions_stream(file, entry)? else {
        info!("sd: jpeg decode fail reason=missing_sof_dims");
        return Ok(None);
    };
    info!(
        "sd: jpeg dims source={}x{} scanned_bytes={}",
        src_w, src_h, scanned_bytes
    );

    let mut thumb_ctx = JpegThumbContext {
        out_bits: out_bits.as_mut_ptr(),
        out_len: dst_bytes,
        thumb_width: tw,
        thumb_height: th,
        source_width: src_w as usize,
        source_height: src_h as usize,
        max_right: 0,
        max_bottom: 0,
        pixels_written: 0,
    };

    let mut jpeg_stream = ZipJpegStream::new(file, entry);
    let io_dispatch = JpegIoDispatch {
        user: (&mut jpeg_stream as *mut _) as *mut c_void,
        read: zip_jpeg_read_dispatch::<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
        skip: zip_jpeg_skip_dispatch::<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    };

    let mut decoder_state = [0u32; JPEG_DECODER_BYTES / core::mem::size_of::<u32>()];
    let mut work = [0u8; JPEG_WORK_BYTES];

    // SAFETY: Callbacks are installed for this decode and guarded by drop cleanup.
    let Some(_guard) = (unsafe {
        JpegCallbackGuard::install(io_dispatch, &mut thumb_ctx as *mut JpegThumbContext)
    }) else {
        info!("sd: jpeg decode fail reason=callback_busy");
        return Ok(None);
    };

    // SAFETY: ROM TJPGD expects opaque state pointer + callbacks + work pool.
    let prep = unsafe {
        jd_prepare(
            decoder_state.as_mut_ptr() as *mut c_void,
            Some(jpeg_in_callback),
            work.as_mut_ptr() as *mut c_void,
            JPEG_WORK_BYTES as u32,
            core::ptr::null_mut(),
        )
    };
    if prep != JDR_OK {
        info!(
            "sd: jpeg decode fail reason=jd_prepare status={} kind={}",
            prep,
            jpeg_jdr_name(prep)
        );
        return Ok(None);
    }

    // Scale=0 keeps full source resolution before thumbnail mapping.
    // SAFETY: Decoder state is initialized by jd_prepare.
    let decomp = unsafe {
        jd_decomp(
            decoder_state.as_mut_ptr() as *mut c_void,
            Some(jpeg_out_callback),
            0,
        )
    };
    if decomp != JDR_OK {
        info!(
            "sd: jpeg decode fail reason=jd_decomp status={} kind={}",
            decomp,
            jpeg_jdr_name(decomp)
        );
        return Ok(None);
    }
    if jpeg_stream.io_failed {
        info!("sd: jpeg decode fail reason=stream_io");
        return Ok(None);
    }
    if thumb_ctx.pixels_written == 0 {
        info!("sd: jpeg decode fail reason=no_pixels");
        return Ok(None);
    }

    if thumb_ctx.max_right.saturating_add(1) < src_w as usize
        || thumb_ctx.max_bottom.saturating_add(1) < src_h as usize
    {
        info!(
            "sd: jpeg decode partial blocks src={}x{} seen={}x{}",
            src_w,
            src_h,
            thumb_ctx.max_right.saturating_add(1),
            thumb_ctx.max_bottom.saturating_add(1)
        );
    }

    Ok(Some((src_w, src_h, dst_bytes)))
}
