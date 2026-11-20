use std::io::{Read, Write};
use zeroize::Zeroizing;

fn stream_v3() {
    use werc_domain::history::{HISTORY_BATCH_BYTES, HistoryReadEvent, HistoryReadRequest};
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let result = (|| -> Result<(), &'static str> {
        let mut prefix = [0u8; 4];
        input
            .read_exact(&mut prefix)
            .map_err(|_| "INVALID_REQUEST")?;
        let length = u32::from_le_bytes(prefix) as usize;
        if length == 0 || length > 65536 {
            return Err("REQUEST_LIMIT_EXCEEDED");
        }
        let mut bytes = Zeroizing::new(vec![0u8; length]);
        input
            .read_exact(&mut bytes)
            .map_err(|_| "INVALID_REQUEST")?;
        let request: HistoryReadRequest =
            serde_json::from_slice(&bytes).map_err(|_| "INVALID_REQUEST")?;
        drop(bytes);
        werc_data_reader::stream_database(&request, &mut |event| {
            let ack = match &event {
                HistoryReadEvent::Begin { .. } => u64::MAX,
                HistoryReadEvent::Batch { sequence, .. } => *sequence,
                HistoryReadEvent::End { .. } => u64::MAX - 1,
                HistoryReadEvent::Error { .. } => return Err("READER_PROTOCOL_INVALID"),
            };
            let bytes =
                Zeroizing::new(serde_json::to_vec(&event).map_err(|_| "OUTPUT_ENCODING_FAILED")?);
            if bytes.len() > HISTORY_BATCH_BYTES + 65536 {
                return Err("RESULT_LIMIT_EXCEEDED");
            }
            output
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .and_then(|_| output.write_all(&bytes))
                .and_then(|_| output.flush())
                .map_err(|_| "READER_PIPE_FAILED")?;
            drop(bytes);
            let mut received = [0u8; 8];
            input
                .read_exact(&mut received)
                .map_err(|_| "READER_ACK_INVALID")?;
            if u64::from_le_bytes(received) != ack {
                return Err("READER_ACK_INVALID");
            }
            Ok(())
        })
    })();
    if let Err(code) = result {
        if let Ok(bytes) = serde_json::to_vec(&HistoryReadEvent::Error { code: code.into() }) {
            let bytes = Zeroizing::new(bytes);
            let _ = output
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .and_then(|_| output.write_all(&bytes))
                .and_then(|_| output.flush());
        }
        // No ACK for errors, including callback/pipe/ACK failures.
        drop(output);
        drop(input);
        std::process::exit(1);
    }
}
fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() == 2 && arguments[1] == "--self-check" {
        println!(
            "{}",
            serde_json::json!({"product":"WeRC_Mon3trBot", "version":env!("CARGO_PKG_VERSION"), "protocol_version":2, "reader":"bounded_read_only_file", "cipher_format":"sqlcipher4_rawkey_4096_sha512", "sqlcipher_engine":false, "key_acquisition_implemented":false, "process_access_performed":false, "real_wechat_verified":false})
        );
        return;
    }
    if arguments.len() == 2 && arguments[1] == "--stream-v3" {
        stream_v3();
        return;
    }
    if arguments.len() != 1 {
        std::process::exit(2);
    }
    let mut input = Zeroizing::new(Vec::new());
    let response = if std::io::stdin()
        .take(65537)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > 65536
    {
        serde_json::json!({"ok":false,"error":"REQUEST_LIMIT_EXCEEDED"})
    } else {
        match serde_json::from_slice::<werc_domain::history::HistoryReadRequest>(&input)
            .map_err(|_| "INVALID_REQUEST")
            .and_then(|r| werc_data_reader::read_database(&r))
        {
            Ok(data) => serde_json::json!({"ok":true,"data":data}),
            Err(code) => serde_json::json!({"ok":false,"error":code}),
        }
    };
    if let Ok(output) = serde_json::to_vec(&response) {
        let output = Zeroizing::new(output);
        let _ = std::io::stdout().write_all(&output);
    }
}
