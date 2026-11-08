use super::*;

pub(crate) struct Compose {
    pub(crate) input: Entity<TextareaState>,
    pub(crate) reply: Option<ReplyTarget>,
    pub(crate) editing: Option<Editing>,
    pub(crate) confirm_delete: Option<u64>,
    pub(in crate::app) delete_closing: Option<Task<()>>,
    pub(crate) failed_sends: HashMap<u64, FailedSend>,
    pub(in crate::app) last_nonce: u64,
    pub(in crate::app) pending_sends: compose::sending::PendingSends,
    pub(in crate::app) drafts: HashMap<u64, ComposerDraft>,
    pub(crate) send_fill: Cell<chat::SendFill>,
    pub(crate) pane_send_fill: Cell<chat::SendFill>,
    pub(in crate::app) _subscription: Subscription,
    pub(in crate::app) _cursor_subscription: Subscription,
}

pub(crate) struct Uploads {
    pub(in crate::app) staged: HashMap<u64, Vec<messages::attachments::StagedFile>>,
    pub(in crate::app) refusal: Option<messages::attachments::Refusal>,
    pub(in crate::app) progress: HashMap<u64, Arc<UploadProgress>>,
    pub(in crate::app) next_id: u64,
    pub(in crate::app) ticking: bool,
    pub(crate) hovered_button: Option<messages::attachments::UploadButton>,
}

pub(crate) struct Suggestions {
    pub(crate) emoji: expressions::emoji_picker::Suggest,
    pub(in crate::app) mention: messages::mentions::Suggest,
    pub(in crate::app) scroll: ScrollHandle,
    pub(in crate::app) mention_names: HashMap<String, u64>,
}

pub(crate) struct History {
    pub(in crate::app) loading: HashSet<u64>,
    pub(in crate::app) older: HashSet<u64>,
    pub(in crate::app) older_retry_at: HashMap<u64, Instant>,
    pub(in crate::app) newer: HashSet<u64>,
    pub(in crate::app) reaction_requests: HashSet<(u64, u64, String)>,
    pub(in crate::app) pending_present: HashSet<u64>,
    pub(in crate::app) pending_jump: Option<u64>,
    pub(in crate::app) failed: HashSet<u64>,
    pub(in crate::app) jump_return: Option<(u64, u64)>,
    pub(in crate::app) complete: HashSet<u64>,
}

pub(crate) struct Jumps {
    pub(in crate::app) scroll: Option<JumpScroll>,
    pub(crate) highlight: Option<(u64, u64)>,
    pub(crate) missing: bool,
    pub(in crate::app) missing_nonce: u64,
}

pub(crate) struct Unread {
    pub(crate) since: Option<u64>,
    pub(crate) bar: bool,
    pub(crate) manual: Option<u64>,
    pub(in crate::app) ack_timer: Option<Task<()>>,
    pub(in crate::app) outgoing_acks: HashMap<u64, Task<()>>,
}

pub(crate) struct Video {
    pub(crate) player: Option<Player>,
    pub(crate) fullscreen: Option<Fullscreen>,
    pub(crate) hovered: Option<u64>,
    pub(in crate::app) drag: Option<(u64, VideoDrag)>,
    pub(in crate::app) dragged_at: Option<Instant>,
    pub(crate) seek_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    pub(crate) volume_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    pub(in crate::app) volume_hover: [bool; 2],
}

pub(crate) struct EmojiPopup {
    pub(crate) picker: Option<expressions::emoji_picker::EmojiPicker>,
    pub(in crate::app) closing: Option<Task<()>>,
    pub(in crate::app) animation: u64,
    pub(in crate::app) closed_at: Option<Instant>,
}

pub(crate) struct Stream {
    pub(crate) view: Option<calls::stream_view::StreamView>,
    pub(in crate::app) stage_chat: Option<(u64, u64)>,
    pub(in crate::app) nonce: u64,
    pub(crate) members_shown: bool,
    pub(crate) members_toggles: u64,
    pub(crate) volume: Option<(Entity<SliderState>, Subscription)>,
    pub(crate) hovered_control: Option<(&'static str, WindowId)>,
}

pub(crate) struct Menus {
    pub(in crate::app) dm: Option<direct::dm_menu::DmMenu>,
    pub(in crate::app) dm_dialog: Option<direct::dm_menu::DmDialog>,
    pub(in crate::app) guild: Option<guilds::guild_menu::GuildMenu>,
    pub(in crate::app) member: Option<members::member_menu::MemberMenu>,
    pub(in crate::app) message_search: Option<search::MessageSearch>,
    pub(in crate::app) forward: Option<forward_dialog::ForwardDialog>,
    pub(in crate::app) reactions: Option<expressions::reactions_dialog::ReactionsDialog>,
    pub(in crate::app) reactions_openings: u64,
    pub(crate) quick_switcher: Option<switcher::QuickSwitcher>,
    pub(in crate::app) quick_switcher_opens: u64,
}

pub(crate) struct Hover {
    pub(crate) message: Option<u64>,
    pub(crate) actions: Option<u64>,
    pub(crate) member: Option<u64>,
    pub(crate) account: bool,
    pub(crate) rail: Option<u64>,
    pub(crate) add_reaction: Option<u64>,
}

pub(crate) struct Game {
    pub(in crate::app) enabled: watch::Sender<bool>,
    pub(in crate::app) generation: u64,
    pub(crate) application: Option<u64>,
    pub(crate) current: Option<String>,
}
