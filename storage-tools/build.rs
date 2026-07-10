fn main() -> Result<(), Box<dyn std::error::Error>> {
    let v1_proto = "proto/zenithstrat/gto/v1/matrix.proto";
    let v2_proto = "proto/zenithstrat/gto/v2/compact_matrix.proto";
    println!("cargo:rerun-if-changed={v1_proto}");
    println!("cargo:rerun-if-changed={v2_proto}");

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::Config::new().compile_protos(&[v1_proto, v2_proto], &["proto"])?;
    Ok(())
}
