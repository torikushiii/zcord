use super::PendingSends;
use crate::app::ReplyTarget;

#[test]
fn staged_sends_cannot_be_bypassed_when_another_response_marks_present() {
    let mut sends = PendingSends::default();
    assert!(sends.should_stage(10, false, false));
    sends.push(10, "A".into(), None, Vec::new(), None);
    assert!(sends.should_stage(10, true, true));
    sends.push(10, "B".into(), None, Vec::new(), None);
    assert!(
        sends.should_stage(10, true, false),
        "the backlog itself is a barrier"
    );
    sends.push(10, "C".into(), None, Vec::new(), None);
    let flushed = sends.take_channel(10);
    assert_eq!(
        flushed
            .iter()
            .map(|entry| entry.1.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert!(
        sends.take_channel(10).is_empty(),
        "a refresh must dispatch each send only once"
    );
    assert!(!sends.should_stage(10, true, false));
}

#[test]
fn failed_refresh_retains_content_and_retries_join_the_tail_without_blocking_other_channels() {
    let mut sends = PendingSends::default();
    let reply = ReplyTarget {
        message_id: 99,
        author_id: 2,
        webhook_name: None,
        mention: false,
    };
    sends.push(10, "A".into(), Some(reply.clone()), Vec::new(), None);
    sends.push(20, "Other".into(), None, Vec::new(), None);
    assert!(sends.should_stage(10, true, false));
    sends.push(10, "B".into(), None, Vec::new(), None);
    sends.push(10, "Retry".into(), Some(reply), Vec::new(), None);
    assert!(!sends.should_stage(30, true, false));
    assert!(
        sends.should_stage(30, true, true),
        "refreshing alone is also a barrier"
    );
    let other = sends.take_channel(20);
    assert_eq!(other[0].1, "Other");
    let flushed = sends.take_channel(10);
    assert_eq!(
        flushed
            .iter()
            .map(|entry| entry.1.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "Retry"]
    );
    let retained = flushed[0].2.as_ref().unwrap();
    assert_eq!(retained.message_id, 99);
    assert!(!retained.mention);
    assert_eq!(flushed[2].2.as_ref().unwrap().message_id, 99);
}

#[test]
fn staged_webhook_reply_retains_its_display_name() {
    let mut sends = PendingSends::default();
    sends.push(
        10,
        "Reply".into(),
        Some(ReplyTarget {
            message_id: 99,
            author_id: 2,
            webhook_name: Some("Webhook display name".into()),
            mention: false,
        }),
        Vec::new(),
        None,
    );
    let flushed = sends.take_channel(10);
    let reply = flushed[0].2.as_ref().unwrap();
    assert_eq!(reply.webhook_name.as_deref(), Some("Webhook display name"));
    assert_eq!(reply.message_id, 99);
    assert_eq!(reply.author_id, 2);
    assert!(!reply.mention);
}
