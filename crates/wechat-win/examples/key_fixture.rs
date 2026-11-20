use std::io::{BufRead, Write};

fn main() {
    if std::env::args().nth(1).as_deref() != Some("--synthetic-memory-target") {
        eprintln!("仅供自有合成取钥测试，必须指定 --synthetic-memory-target");
        std::process::exit(2);
    }
    let needle = Box::new(*b"com.Tencent.WCDB.Config.Cipher");
    let mask =
        hex::decode("d2c7442458020000004889442450488b450048844c2448488944254048584c24").unwrap();
    let raw = format!("x'{}{}'", "5a".repeat(32), "35".repeat(16));
    let blob = raw
        .bytes()
        .enumerate()
        .map(|(i, b)| b ^ mask[i % mask.len()])
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let mut config = Box::new([0u8; 0xb0]);
    config[0x90..0x98].copy_from_slice(&(blob.as_ptr() as u64).to_le_bytes());
    config[0x98..0xa0].copy_from_slice(&(blob.len() as u64).to_le_bytes());
    let mut node = Box::new([0u8; 0x50]);
    node[0x10..0x18].copy_from_slice(&(needle.as_ptr() as u64).to_le_bytes());
    node[0x18..0x20].copy_from_slice(&(needle.len() as u64).to_le_bytes());
    node[0x28..0x30].copy_from_slice(&(config.as_ptr() as u64).to_le_bytes());
    std::hint::black_box((&node, &config, &needle, &blob));
    println!("SYNTHETIC_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).unwrap();
    std::hint::black_box((&node, &config, &needle, &blob));
}
