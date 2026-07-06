fn main() {
    // Generate Rust types from the protobuf schema shared with the old Go app.
    prost_build::compile_protos(&["model/message.proto"], &["model"])
        .expect("failed to compile model/message.proto");
    println!("cargo:rerun-if-changed=model/message.proto");
}
