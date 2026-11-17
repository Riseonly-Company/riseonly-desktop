use std::any::Any;
use std::collections::HashSet;

use rise_canon::rise_key::{comment as comment_key, message as message_key, post as post_key};
use serde_json::{Map, Value};

use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(name, value)| (name.to_owned(), value)).collect::<Map<String, Value>>())
}

fn author(id: &str) -> Value {
    object(vec![(post_key::ID, Value::from(id))])
}

#[test]
fn own_profile_projection_rejects_foreign_rows() {
    let payload = object(vec![
        (
            post_key::LIST,
            Value::Array(vec![
                object(vec![
                    (post_key::ID, Value::from(1)),
                    (post_key::TITLE, Value::from("own")),
                    (post_key::AUTHOR, author("me")),
                ]),
                object(vec![
                    (post_key::ID, Value::from(2)),
                    (post_key::TITLE, Value::from("foreign")),
                    (post_key::AUTHOR, author("other")),
                ]),
                object(vec![
                    (post_key::ID, Value::from(3)),
                    (post_key::TITLE, Value::from("foreign media")),
                    (post_key::IMAGES, Value::from(vec!["https://foreign.example/repost.jpg"])),
                    (post_key::AUTHOR, author("other")),
                ]),
                object(vec![
                    (post_key::ID, Value::from(4)),
                    (post_key::TITLE, Value::from("foreign video")),
                    (post_key::VIDEO, Value::from("https://foreign.example/video.mp4")),
                    (post_key::AUTHOR, author("other")),
                ]),
            ]),
        ),
        (post_key::IS_HAVE_MORE, Value::Bool(true)),
    ]);

    let page = PostServicesStore::sanitized_own_user_posts_page(Some(&payload as &dyn Any), |post| {
        post.author.id.as_deref() == Some("me")
    })
    .expect("the payload decodes as a feed page");

    assert_eq!(page.list.iter().map(|post| post.id.as_str()).collect::<Vec<_>>(), vec!["1"]);
    assert_eq!(page.relative_id.as_deref(), Some("4"));
    assert_eq!(page.is_have_more, Some(true));
}

#[test]
fn message_disk_copy_redacts_foreign_shared_post_but_keeps_own_shared_post() {
    let payload = object(vec![(
        message_key::MESSAGES,
        Value::Array(vec![
            object(vec![
                (message_key::ID, Value::from("foreign-message")),
                (message_key::CONTENT_TYPE, Value::from("post_short")),
                (
                    message_key::SHARED_POST,
                    object(vec![
                        (message_key::POST_ID, Value::from(101)),
                        (comment_key::IS_SHORT, Value::Bool(true)),
                        (message_key::TITLE, Value::from("foreign title")),
                        (message_key::SNIPPET, Value::from("foreign snippet")),
                        (post_key::IMAGES, Value::from(vec!["https://foreign.example/image.jpg"])),
                        (post_key::VIDEO, Value::from("https://foreign.example/video.mp4")),
                        (comment_key::VIDEO_THUMBNAIL, Value::from("https://foreign.example/thumb.jpg")),
                        (
                            post_key::AUTHOR,
                            object(vec![
                                (message_key::ID, Value::from("other")),
                                (message_key::NAME, Value::from("Foreign Author")),
                                (message_key::LOGO, Value::from("https://foreign.example/avatar.jpg")),
                            ]),
                        ),
                    ]),
                ),
            ]),
            object(vec![
                (message_key::ID, Value::from("own-message")),
                (message_key::CONTENT_TYPE, Value::from(message_key::SHARED_POST)),
                (
                    message_key::SHARED_POST,
                    object(vec![
                        (message_key::POST_ID, Value::from(202)),
                        (comment_key::IS_SHORT, Value::Bool(false)),
                        (message_key::TITLE, Value::from("own title")),
                        (post_key::IMAGES, Value::from(vec!["https://own.example/image.jpg"])),
                        (
                            post_key::AUTHOR,
                            object(vec![
                                (message_key::ID, Value::from("me")),
                                (message_key::NAME, Value::from("Me")),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ]),
    )]);

    let sanitized = MessageServicesStore::sanitized_messages_payload_for_disk(&payload, |id| id == Some("me"));
    let foreign = sanitized[message_key::MESSAGES][0][message_key::SHARED_POST]
        .as_object()
        .expect("the foreign shared post stays an object");
    let own = &sanitized[message_key::MESSAGES][1][message_key::SHARED_POST];

    assert_eq!(
        foreign.keys().map(String::as_str).collect::<HashSet<_>>(),
        HashSet::from([message_key::POST_ID, comment_key::IS_SHORT])
    );
    assert_eq!(foreign[message_key::POST_ID].as_i64(), Some(101));
    assert_eq!(foreign[comment_key::IS_SHORT].as_bool(), Some(true));
    assert_eq!(own[message_key::TITLE].as_str(), Some("own title"));
    assert_eq!(own[post_key::IMAGES], Value::from(vec!["https://own.example/image.jpg"]));

    let original_foreign = &payload[message_key::MESSAGES][0][message_key::SHARED_POST];
    assert_eq!(original_foreign[message_key::TITLE].as_str(), Some("foreign title"));
}
