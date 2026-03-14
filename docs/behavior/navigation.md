# Chapter And Paragraph Navigation

This note documents the current navigation flow once a book is paused.

## Entry Conditions

Navigation is entered from paused reading only.

Current trigger:

- Pause the reader with a single press.
- Rotate in either direction while paused.

The app enters chapter navigation first. Paragraph selection is always a second step.

Current limitation:

- There is no dedicated cancel or back gesture inside chapter or paragraph selection. Rotation only
  changes the target, and press always drills down or confirms.

## Chapter Selector

The chapter selector compares the current chapter and a target chapter.

Current behavior:

- Rotation wraps across the available chapter count.
- The UI shows current and target labels, plus chapter indices.
- For stream-backed books, only the current chapter has real paragraph metadata loaded.
- Unloaded chapters can appear as placeholder `Chapter N` entries until the target chapter data is
  fetched.

Press behavior:

- Press on a valid target chapter requests `seek_chapter(target)`.
- If the chapter data is already available, the app goes straight to paragraph navigation.
- If the chapter data is not ready yet, the app enters the chapter-loading screen and waits for the
  refill pipeline to populate the target chapter.

Current visible limitation:

- Large EPUB chapter jumps can be slow because the refill path may need multiple probe steps to
  reach the requested resource.

## Chapter Loading

`NavigateChapterLoading` is a real UI state.

Current behavior:

- The screen keeps showing current and target chapter context.
- Input is ignored while the chapter is loading.
- The state exits automatically once `chapter_data_ready(target)` becomes true.

## Paragraph Selector

Once a chapter is ready, the app enters paragraph navigation for that chapter.

Current behavior:

- Rotation wraps within the paragraphs currently available for the selected chapter.
- The selector shows the chapter label, a current position label, and a preview of the target
  paragraph.
- Press confirms the target paragraph and returns to reading in a paused state.

After confirmation:

- The app seeks to the target paragraph.
- Reading word state is reset.
- The first word is preloaded.
- The UI returns to paused reading so the user can inspect the new position before resuming.

## Failure Cases

- Invalid chapter index: `NAVIGATION ERROR / CHAPTER INVALID`
- Chapter seek failure: `NAVIGATION ERROR / CHAPTER SEEK FAILED`
- Invalid paragraph target: `NAVIGATION ERROR / PARAGRAPH INVALID`

The app uses the generic status screen for these failures and returns to the library on press.
