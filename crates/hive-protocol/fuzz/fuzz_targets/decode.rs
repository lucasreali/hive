#![no_main]

use bytes::BytesMut;
use hive_protocol::FrameCodec;
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::Decoder;

// The decoder must never panic: it returns frames, waits for more bytes, or fails with an error.
fuzz_target!(|data: &[u8]| {
    let mut buf = BytesMut::from(data);
    while let Ok(Some(frame)) = FrameCodec.decode(&mut buf) {
        let _ = frame.to_control();
    }
});
