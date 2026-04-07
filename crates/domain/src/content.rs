use alloc::boxed::Box;

use crate::{source::SourceKind, text::InlineText};

pub const ARTICLE_COUNT_PER_COLLECTION: usize = 5;
pub const MANIFEST_ITEM_CAPACITY: usize = 16;
pub const PARAGRAPH_COUNT_PER_SCRIPT: usize = 23;
pub const CONTENT_META_MAX_BYTES: usize = 48;
pub const CONTENT_TITLE_MAX_BYTES: usize = 96;
pub const CONTENT_ID_MAX_BYTES: usize = 36;
pub const REMOTE_ITEM_ID_MAX_BYTES: usize = 36;
pub const RECOMMENDATION_SERVE_ID_MAX_BYTES: usize = 36;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum CollectionKind {
    #[default]
    Saved,
    Inbox,
    Recommendations,
}

impl CollectionKind {
    pub const ALL: [Self; 3] = [Self::Inbox, Self::Saved, Self::Recommendations];

    pub const fn dashboard_label(self) -> &'static str {
        match self {
            Self::Saved => "SAVED",
            Self::Inbox => "INBOX",
            Self::Recommendations => "FOR YOU",
        }
    }

    pub const fn rail_label(self) -> &'static str {
        match self {
            Self::Saved => "S\nA\nV\nE\nD",
            Self::Inbox => "I\nN\nB\nO\nX",
            Self::Recommendations => "F\nO\nR\n\nY\nO\nU",
        }
    }

    pub const fn has_dashboard_live_dot(self) -> bool {
        !matches!(self, Self::Saved)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReaderScript {
    MachineSoul,
    QuietCraft,
    PortableAttention,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ArticleId(pub u16);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ArticleSummary {
    pub id: ArticleId,
    pub source: SourceKind,
    pub meta: &'static str,
    pub title: &'static str,
    pub reader_title: &'static str,
    pub reader_preview: &'static str,
    pub chat_preview: &'static str,
    pub reader_left_word: &'static str,
    pub reader_right_word: &'static str,
    pub script: ReaderScript,
    pub has_chat: bool,
}

impl ArticleSummary {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        id: ArticleId,
        source: SourceKind,
        meta: &'static str,
        title: &'static str,
        reader_title: &'static str,
        reader_preview: &'static str,
        chat_preview: &'static str,
        reader_left_word: &'static str,
        reader_right_word: &'static str,
        script: ReaderScript,
        has_chat: bool,
    ) -> Self {
        Self {
            id,
            source,
            meta,
            title,
            reader_title,
            reader_preview,
            chat_preview,
            reader_left_word,
            reader_right_word,
            script,
            has_chat,
        }
    }
}

impl Default for ArticleSummary {
    fn default() -> Self {
        Self::new(
            ArticleId(0),
            SourceKind::Unknown,
            "MOTIF / 00.APR",
            "Mock article title",
            "THE MACHINE SOUL",
            "Analog objects still teach us",
            "I think we should keep",
            "PU",
            "LSE",
            ReaderScript::MachineSoul,
            true,
        )
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DetailLocator {
    Saved,
    Inbox,
    Content,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum RemoteContentStatus {
    Ready,
    Pending,
    Failed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum PackageState {
    #[default]
    Missing,
    Cached,
    Stale,
    Fetching,
    PendingRemote,
    Failed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CollectionManifestItem {
    pub remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    pub content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    pub detail_locator: DetailLocator,
    pub source: SourceKind,
    pub meta: InlineText<CONTENT_META_MAX_BYTES>,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub remote_revision: u64,
    pub remote_status: RemoteContentStatus,
    pub package_state: PackageState,
}

impl CollectionManifestItem {
    pub const fn empty() -> Self {
        Self {
            remote_item_id: InlineText::new(),
            content_id: InlineText::new(),
            detail_locator: DetailLocator::Saved,
            source: SourceKind::Unknown,
            meta: InlineText::new(),
            title: InlineText::new(),
            remote_revision: 0,
            remote_status: RemoteContentStatus::Unknown,
            package_state: PackageState::Missing,
        }
    }

    pub const fn is_cached(self) -> bool {
        matches!(self.package_state, PackageState::Cached)
    }

    pub const fn can_prepare(self) -> bool {
        matches!(self.remote_status, RemoteContentStatus::Ready)
            && matches!(
                self.package_state,
                PackageState::Missing | PackageState::Stale | PackageState::Failed
            )
    }
}

impl Default for CollectionManifestItem {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CollectionManifestState {
    pub items: [CollectionManifestItem; MANIFEST_ITEM_CAPACITY],
    pub serve_id: InlineText<RECOMMENDATION_SERVE_ID_MAX_BYTES>,
    len: u8,
}

impl CollectionManifestState {
    pub const fn empty() -> Self {
        Self {
            items: [CollectionManifestItem::empty(); MANIFEST_ITEM_CAPACITY],
            serve_id: InlineText::new(),
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        *self = Self::empty();
    }

    pub fn try_push(&mut self, item: CollectionManifestItem) -> bool {
        let len = self.len();
        if len >= MANIFEST_ITEM_CAPACITY {
            return false;
        }

        self.items[len] = item;
        self.len = self.len.saturating_add(1);
        true
    }

    pub fn item_at(&self, index: usize) -> Option<CollectionManifestItem> {
        if self.is_empty() {
            None
        } else {
            Some(self.items[index % self.len()])
        }
    }

    pub fn item_mut_at(&mut self, index: usize) -> Option<&mut CollectionManifestItem> {
        if self.is_empty() {
            None
        } else {
            let len = self.len();
            Some(&mut self.items[index % len])
        }
    }

    pub fn update_package_state(
        &mut self,
        remote_item_id: &InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        package_state: PackageState,
    ) -> bool {
        let len = self.len();
        let mut index = 0;
        while index < len {
            if self.items[index].remote_item_id == *remote_item_id {
                self.items[index].package_state = package_state;
                return true;
            }
            index += 1;
        }

        false
    }
}

impl Default for CollectionManifestState {
    fn default() -> Self {
        Self::empty()
    }
}

const EMPTY_COLLECTION_STATE: CollectionManifestState = CollectionManifestState::empty();

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ContentState {
    pub saved: Option<Box<CollectionManifestState>>,
    pub inbox: Option<Box<CollectionManifestState>>,
    pub recommendations: Option<Box<CollectionManifestState>>,
}

impl ContentState {
    pub const fn empty() -> Self {
        Self {
            saved: None,
            inbox: None,
            recommendations: None,
        }
    }

    pub fn boxed_empty() -> Box<Self> {
        Box::new(Self::empty())
    }

    pub const fn collection_len(&self, kind: CollectionKind) -> usize {
        self.collection_state(kind).len()
    }

    pub const fn collection_state(&self, kind: CollectionKind) -> &CollectionManifestState {
        match kind {
            CollectionKind::Saved => match &self.saved {
                Some(collection) => collection,
                None => &EMPTY_COLLECTION_STATE,
            },
            CollectionKind::Inbox => match &self.inbox {
                Some(collection) => collection,
                None => &EMPTY_COLLECTION_STATE,
            },
            CollectionKind::Recommendations => match &self.recommendations {
                Some(collection) => collection,
                None => &EMPTY_COLLECTION_STATE,
            },
        }
    }

    pub fn collection_state_mut(&mut self, kind: CollectionKind) -> &mut CollectionManifestState {
        match kind {
            CollectionKind::Saved => self
                .saved
                .get_or_insert_with(|| Box::new(CollectionManifestState::empty()))
                .as_mut(),
            CollectionKind::Inbox => self
                .inbox
                .get_or_insert_with(|| Box::new(CollectionManifestState::empty()))
                .as_mut(),
            CollectionKind::Recommendations => self
                .recommendations
                .get_or_insert_with(|| Box::new(CollectionManifestState::empty()))
                .as_mut(),
        }
    }

    pub fn manifest_item_at(
        &self,
        kind: CollectionKind,
        index: usize,
    ) -> Option<CollectionManifestItem> {
        self.collection_state(kind).item_at(index)
    }

    pub fn update_collection(&mut self, kind: CollectionKind, collection: CollectionManifestState) {
        if collection.is_empty() {
            self.clear_collection(kind);
        } else {
            *self.collection_state_mut(kind) = collection;
        }
    }

    pub fn update_boxed_collection(
        &mut self,
        kind: CollectionKind,
        collection: Box<CollectionManifestState>,
    ) {
        let slot = self.collection_slot_mut(kind);
        if collection.is_empty() {
            *slot = None;
        } else {
            *slot = Some(collection);
        }
    }

    pub fn update_package_state(
        &mut self,
        kind: CollectionKind,
        remote_item_id: &InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        package_state: PackageState,
    ) -> bool {
        self.collection_state_mut(kind)
            .update_package_state(remote_item_id, package_state)
    }

    pub fn article_at(&self, kind: CollectionKind, index: usize) -> ArticleSummary {
        match kind {
            CollectionKind::Saved => SAVED_ARTICLES[index % ARTICLE_COUNT_PER_COLLECTION],
            CollectionKind::Inbox => INBOX_ARTICLES[index % ARTICLE_COUNT_PER_COLLECTION],
            CollectionKind::Recommendations => {
                RECOMMENDATION_ARTICLES[index % ARTICLE_COUNT_PER_COLLECTION]
            }
        }
    }

    pub fn article_by_id(&self, kind: CollectionKind, article_id: ArticleId) -> ArticleSummary {
        let collection = match kind {
            CollectionKind::Saved => &SAVED_ARTICLES,
            CollectionKind::Inbox => &INBOX_ARTICLES,
            CollectionKind::Recommendations => &RECOMMENDATION_ARTICLES,
        };
        let mut index = 0;

        while index < collection.len() {
            if collection[index].id == article_id {
                return collection[index];
            }
            index += 1;
        }

        collection[0]
    }

    fn clear_collection(&mut self, kind: CollectionKind) {
        *self.collection_slot_mut(kind) = None;
    }

    fn collection_slot_mut(
        &mut self,
        kind: CollectionKind,
    ) -> &mut Option<Box<CollectionManifestState>> {
        match kind {
            CollectionKind::Saved => &mut self.saved,
            CollectionKind::Inbox => &mut self.inbox,
            CollectionKind::Recommendations => &mut self.recommendations,
        }
    }
}

impl Default for ContentState {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PrepareContentRequest {
    pub collection: CollectionKind,
    pub remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    pub content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    pub detail_locator: DetailLocator,
    pub remote_revision: u64,
}

impl PrepareContentRequest {
    pub const fn from_manifest(collection: CollectionKind, item: CollectionManifestItem) -> Self {
        Self {
            collection,
            remote_item_id: item.remote_item_id,
            content_id: item.content_id,
            detail_locator: item.detail_locator,
            remote_revision: item.remote_revision,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum PrepareContentPhase {
    #[default]
    Connecting,
    Downloading,
    Caching,
    Opening,
}

impl PrepareContentPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "CONNECTING",
            Self::Downloading => "DOWNLOADING",
            Self::Caching => "CACHING",
            Self::Opening => "OPENING",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PrepareContentProgress {
    pub phase: PrepareContentPhase,
    pub completed_steps: u16,
    pub total_steps: u16,
}

impl PrepareContentProgress {
    pub const fn connecting() -> Self {
        Self {
            phase: PrepareContentPhase::Connecting,
            completed_steps: 0,
            total_steps: 4,
        }
    }

    pub const fn progress_width_px(self, max_width_px: u16) -> u16 {
        if self.total_steps == 0 {
            return 0;
        }

        let completed_steps = if self.completed_steps > self.total_steps {
            self.total_steps
        } else {
            self.completed_steps
        };

        ((max_width_px as u32 * completed_steps as u32) / self.total_steps as u32) as u16
    }
}

impl Default for PrepareContentProgress {
    fn default() -> Self {
        Self::connecting()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ArticleDocument {
    pub source: SourceKind,
    pub script: ReaderScript,
}

impl ArticleDocument {
    pub const fn new(source: SourceKind, script: ReaderScript) -> Self {
        Self { source, script }
    }
}

impl Default for ArticleDocument {
    fn default() -> Self {
        Self::new(SourceKind::Unknown, ReaderScript::MachineSoul)
    }
}

pub const fn script_paragraph_count(_script: ReaderScript) -> usize {
    PARAGRAPH_COUNT_PER_SCRIPT
}

pub fn script_paragraph(script: ReaderScript, index: usize) -> &'static str {
    let clamped = index.min(PARAGRAPH_COUNT_PER_SCRIPT.saturating_sub(1));

    match script {
        ReaderScript::MachineSoul => MACHINE_SOUL_PARAGRAPHS[clamped],
        ReaderScript::QuietCraft => QUIET_CRAFT_PARAGRAPHS[clamped],
        ReaderScript::PortableAttention => PORTABLE_ATTENTION_PARAGRAPHS[clamped],
    }
}

const SAVED_ARTICLES: [ArticleSummary; ARTICLE_COUNT_PER_COLLECTION] = [
    ArticleSummary::new(
        ArticleId(101),
        SourceKind::PersonalQueue,
        "THE VERGE / 25.OCT",
        "The future of analog hardware\nin a silicon world",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
    ArticleSummary::new(
        ArticleId(102),
        SourceKind::PersonalQueue,
        "WIRED / 24.OCT",
        "Post-digital: why we crave the\ntactile click of physical tools",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
    ArticleSummary::new(
        ArticleId(103),
        SourceKind::PersonalQueue,
        "PITCHFORK / 22.OCT",
        "Synthesizing the wind: Ambient\nrecordings of 1974",
        "QUIET CRAFT",
        "A soft tool can still be exact.",
        "The slower gesture is the one",
        "SO",
        "FT",
        ReaderScript::QuietCraft,
        true,
    ),
    ArticleSummary::new(
        ArticleId(104),
        SourceKind::PersonalQueue,
        "MONOCLE / 18.OCT",
        "Portable studios and the return\nof private attention",
        "PORTABLE ATTENTION",
        "Saved pages should open before",
        "We can treat the queue as a",
        "FO",
        "CUSED",
        ReaderScript::PortableAttention,
        true,
    ),
    ArticleSummary::new(
        ArticleId(105),
        SourceKind::PersonalQueue,
        "MOTIF / 15.OCT",
        "Designing a reading device that\nnever begs for a notification",
        "PORTABLE ATTENTION",
        "Saved pages should open before",
        "We can treat the queue as a",
        "FO",
        "CUSED",
        ReaderScript::PortableAttention,
        true,
    ),
];

const INBOX_ARTICLES: [ArticleSummary; ARTICLE_COUNT_PER_COLLECTION] = [
    ArticleSummary::new(
        ArticleId(201),
        SourceKind::EditorialFeed,
        "THE VERGE / 25.OCT",
        "The future of analog hardware\nin a silicon world",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
    ArticleSummary::new(
        ArticleId(202),
        SourceKind::EditorialFeed,
        "WIRED / 24.OCT",
        "Post-digital: why we crave the\ntactile click of physical tools",
        "QUIET CRAFT",
        "A soft tool can still be exact.",
        "The slower gesture is the one",
        "SO",
        "FT",
        ReaderScript::QuietCraft,
        true,
    ),
    ArticleSummary::new(
        ArticleId(203),
        SourceKind::EditorialFeed,
        "PITCHFORK / 22.OCT",
        "Synthesizing the wind: Ambient\nrecordings of 1974",
        "PORTABLE ATTENTION",
        "Saved pages should open before",
        "We can treat the queue as a",
        "FO",
        "CUSED",
        ReaderScript::PortableAttention,
        true,
    ),
    ArticleSummary::new(
        ArticleId(204),
        SourceKind::EditorialFeed,
        "MOTIF / 19.OCT",
        "An honest display for long-form\nreading in daylight",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
    ArticleSummary::new(
        ArticleId(205),
        SourceKind::EditorialFeed,
        "MONOCLE / 17.OCT",
        "The weight of a hinge and other\nnotes on deliberate objects",
        "QUIET CRAFT",
        "A soft tool can still be exact.",
        "The slower gesture is the one",
        "SO",
        "FT",
        ReaderScript::QuietCraft,
        true,
    ),
];

const RECOMMENDATION_ARTICLES: [ArticleSummary; ARTICLE_COUNT_PER_COLLECTION] = [
    ArticleSummary::new(
        ArticleId(301),
        SourceKind::EditorialFeed,
        "THE VERGE / 25.OCT",
        "The future of analog hardware\nin a silicon world",
        "PORTABLE ATTENTION",
        "Saved pages should open before",
        "We can treat the queue as a",
        "FO",
        "CUSED",
        ReaderScript::PortableAttention,
        true,
    ),
    ArticleSummary::new(
        ArticleId(302),
        SourceKind::EditorialFeed,
        "WIRED / 24.OCT",
        "Post-digital: why we crave the\ntactile click of physical tools",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
    ArticleSummary::new(
        ArticleId(303),
        SourceKind::EditorialFeed,
        "PITCHFORK / 22.OCT",
        "Synthesizing the wind: Ambient\nrecordings of 1974",
        "QUIET CRAFT",
        "A soft tool can still be exact.",
        "The slower gesture is the one",
        "SO",
        "FT",
        ReaderScript::QuietCraft,
        true,
    ),
    ArticleSummary::new(
        ArticleId(304),
        SourceKind::EditorialFeed,
        "MOTIF / 20.OCT",
        "Why a device can feel calm when\nits defaults stay out of the way",
        "PORTABLE ATTENTION",
        "Saved pages should open before",
        "We can treat the queue as a",
        "FO",
        "CUSED",
        ReaderScript::PortableAttention,
        true,
    ),
    ArticleSummary::new(
        ArticleId(305),
        SourceKind::EditorialFeed,
        "MONOCLE / 18.OCT",
        "A portable-reading collection you\ncan live inside all week",
        "THE MACHINE SOUL",
        "Analog objects still teach us",
        "I think we should keep",
        "PU",
        "LSE",
        ReaderScript::MachineSoul,
        true,
    ),
];

const MACHINE_SOUL_PARAGRAPHS: [&str; PARAGRAPH_COUNT_PER_SCRIPT] = [
    "A machine can feel human when the pacing is generous.",
    "Fast systems become legible once the interface stops shouting.",
    "The panel only feels slow when the transition has no point.",
    "Objects become memorable when their rhythm is visible.",
    "A deliberate device has to earn every black pixel it lights.",
    "The screen disappears when the cadence holds and there's nothing left to chase.",
    "Analog objects still teach us what speed tends to erase.",
    "A soft tool can still be exact.",
    "The reader only feels lost when the paragraph map is gone.",
    "Mechanical cues give software somewhere firm to stand.",
    "The diagonal bar reads like an action instead of decoration.",
    "Clarity arrives when motion describes the state transition.",
    "Fast feedback matters more than theoretical frame counts.",
    "The queue should feel staged, not dumped.",
    "Saved reading must open ready before it opens clever.",
    "A visible pause is part of the reading instrument.",
    "Chat belongs beside the sentence, never over it.",
    "Progress should advance like a mark on paper.",
    "The best overlay is one that knows when to leave.",
    "Paragraph maps keep the RSVP stream from turning abstract.",
    "Small screens need stronger hierarchy, not more chrome.",
    "Motion should latch the next state into place.",
    "A calm device still needs theatrical focus moves.",
];

const QUIET_CRAFT_PARAGRAPHS: [&str; PARAGRAPH_COUNT_PER_SCRIPT] = [
    "Craft starts with removing the dramatic gesture that does not help.",
    "Quiet tools earn trust by revealing their internal logic, and I'd keep that rule visible.",
    "Materials feel richer when the interface leaves room around them.",
    "Weight and friction become part of the narrative on contact.",
    "Precision can look calm without becoming bland.",
    "A hinge describes intention long before it completes the motion.",
    "A brush line can be exact without pretending to be sterile.",
    "Slow transitions let the hand understand what changed.",
    "A pause should feel held, not frozen.",
    "Feedback works better when it resolves into structure.",
    "Monochrome reveals weak composition almost immediately.",
    "Dense words need generous staging on small panels.",
    "The best control is the one whose movement teaches the rule.",
    "Reading hardware should honor the tempo of a page turn.",
    "A gentle pulse can be clearer than a fast blink.",
    "Composition is how a tool admits its limits.",
    "Menus become friendlier when each step lands with conviction.",
    "Precision is as much timing as geometry.",
    "Deliberate interfaces keep the hand from over-correcting.",
    "Exactness gets easier when the path is visible.",
    "State transitions should feel like assembled parts.",
    "A reader can be both mechanical and intimate.",
    "Quiet confidence is still a visual style.",
];

const PORTABLE_ATTENTION_PARAGRAPHS: [&str; PARAGRAPH_COUNT_PER_SCRIPT] = [
    "Portable reading begins by protecting the first minute.",
    "Saved pages should open before the user remembers why they saved them, because that's the promise.",
    "A queue becomes personal once it keeps your place everywhere.",
    "Offline-first is mostly a promise about emotional continuity.",
    "The best device handoff is the one you do not notice.",
    "Lists need enough structure to be skimmed without becoming noisy.",
    "Editorial picks should feel adjacent to your own queue, not distant.",
    "Recommendations are strongest when they still look like reading.",
    "A focused device should never demand the whole network to feel alive.",
    "Motion can suggest freshness without imitating a phone.",
    "Context belongs in the margins, not in front of the sentence.",
    "A queue card should summarize, not explain.",
    "The vertical rail keeps the screen from feeling generic.",
    "Progress markers matter because return paths matter.",
    "The home screen is a promise about where the night will begin.",
    "A good dashboard sets tone before it sets options.",
    "The right amount of delay can make a transition legible.",
    "Portable attention is not the same as portable distraction.",
    "The reader should feel confident even when Wi-Fi disappears.",
    "Topic tuning works best when it looks like curation, not settings.",
    "A refresh action can feel ceremonial on a dedicated device.",
    "The content list is the product, not a hallway.",
    "Saved reading should feel ready for tonight.",
];
