use serde::Deserialize;
use std::str::FromStr;

// TODO: investigate if its worth having OS(platform) versus a single unified list
// where each architecture and operating system are combined. Or answer the question if
// architecture optimizations can be shared across operating systems. Can we make the same
// optimizations for a m1 that we can for a arm linux laptop?

/// The is the complilation target platform.
#[derive(Debug, Clone, Deserialize)]
pub enum TargetPlatform {
    /// x86_64-unknown-linux-gnu
    UnknownLinuxAmd64,
    /// aarch64-unknown-linux-gnu
    UnknownLinuxArm64,
    /// aarch64-apple-darwin
    AppleDarwinArm64,
    /// x86_64-apple-darwin
    AppleDarwinAmd64,
}

impl FromStr for TargetPlatform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let platform = match s {
            "x86_64-unknown-linux-gnu" => TargetPlatform::UnknownLinuxAmd64,
            "aarch64-unknown-linux-gnu" => TargetPlatform::UnknownLinuxArm64,
            "aarch64-apple-darwin" => TargetPlatform::AppleDarwinArm64,
            // "x86_64-apple-darwin" => TargetPlatform::AppleDarwinAmd64,
            _ => return Err("unsupported target platform".to_string()),
        };

        Ok(platform)
    }
}
