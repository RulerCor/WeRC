fn main() {
    println!("cargo:rerun-if-env-changed=WERC_EMOTICON_CODEC_SHA256");
    println!("cargo:rerun-if-env-changed=WERC_WORKER_SHA256");
    println!("cargo:rerun-if-env-changed=WERC_MESSAGE_HOST_SHA256");
    println!("cargo:rerun-if-env-changed=WERC_VOICE_CODEC_SHA256");
}
