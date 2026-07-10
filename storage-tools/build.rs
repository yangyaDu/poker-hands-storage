fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/zenithstrat/gto/v1/matrix.proto";
    println!("cargo:rerun-if-changed={proto}");

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    prost_build::Config::new().compile_protos(&[proto], &["proto"])?;
    Ok(())
}
