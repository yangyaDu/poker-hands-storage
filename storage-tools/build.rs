fn main() -> Result<(), Box<dyn std::error::Error>> {
    let v2_proto = "proto/zenithstrat/gto/v2/compact_matrix.proto";
    let v3_proto = "proto/poker/hands/storage/v3/compact_range_storage.proto";
    println!("cargo:rerun-if-changed={v2_proto}");
    println!("cargo:rerun-if-changed={v3_proto}");

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::Config::new().compile_protos(&[v2_proto, v3_proto], &["proto"])?;
    Ok(())
}
