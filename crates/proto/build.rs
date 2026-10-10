fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::compile_protos("proto/daemon.v1.proto")?;

    Ok(())
}
