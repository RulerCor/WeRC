//! Owned protocol fault fixture. Never opens a database or sends a message.
use std::{
    io::{Read, Write},
    time::Duration,
};
fn main() {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .unwrap();
    let input: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mode = std::path::Path::new(input["path"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    match mode {
        "sender-empty" | "sender-control" | "sender-oversized" | "sender-valid"
        | "sender-legacy" => {
            let mut row = serde_json::json!({"local_id":"1","server_id":null,"timestamp":1,"message_type":"1","text":"owned","content_status":"text"});
            if mode != "sender-legacy" {
                row["sender_username"] = serde_json::json!(match mode {
                    "sender-empty" => String::new(),
                    "sender-control" => "owned\0sender".into(),
                    "sender-oversized" => "x".repeat(257),
                    _ => "owned-sender".into(),
                });
            }
            print!(
                "{}",
                serde_json::json!({"ok":true,"data":{"schema_version":1,"rows":[row],"has_older":false,"read_only":true,"readonly_shm":true,"read_mode":"bounded_transaction_window"}})
            );
        }
        "image-valid"
        | "video-valid"
        | "emoticon-valid"
        | "emoticon-wrong-type"
        | "emoticon-oversized"
        | "emoticon-mixed-text" => {
            let mut row = serde_json::json!({"local_id":"1","server_id":null,"timestamp":1,"message_type":"47","text":null,"metadata":"<msg><emoji/></msg>","content_status":"metadata"});
            if mode == "image-valid" {
                row["message_type"] = serde_json::json!("3");
                row["metadata"] = serde_json::json!("<msg><img/></msg>");
            }
            if mode == "video-valid" {
                row["message_type"] = serde_json::json!("43");
                row["metadata"] = serde_json::json!("<msg><videomsg/></msg>");
            }
            if mode == "emoticon-wrong-type" {
                row["message_type"] = serde_json::json!("34");
            }
            if mode == "emoticon-oversized" {
                row["metadata"] = serde_json::json!("x".repeat(16385));
            }
            if mode == "emoticon-mixed-text" {
                row["text"] = serde_json::json!("never render");
            }
            print!(
                "{}",
                serde_json::json!({"ok":true,"data":{"schema_version":1,"rows":[row],"has_older":false,"read_only":true,"readonly_shm":true,"read_mode":"bounded_transaction_window"}})
            );
        }
        "invalid" => print!("invalid-json"),
        "oversized" => {
            let _ = std::io::stdout().write_all(&vec![b'x'; 65537]);
        }
        "crash" => std::process::exit(17),
        "cancel" | "timeout" => loop {
            std::thread::sleep(Duration::from_millis(100));
        },
        "valid" => print!(
            "{{\"ok\":true,\"data\":{{\"schema_objects\":2,\"database_pages\":5,\"cipher_version\":\"owned-fixture\",\"sqlite_version\":\"owned-fixture\",\"read_only\":true,\"readonly_shm\":true,\"immutable\":false,\"rows_exported\":0,\"full_g1a_verified\":false}}}}"
        ),
        _ => std::process::exit(2),
    }
}
