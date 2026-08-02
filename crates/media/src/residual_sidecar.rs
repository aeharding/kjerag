//! Versioned, fail-closed storage for research source-coordinate residuals.

use std::io::{self, Read};

pub const MAGIC: [u8; 8] = *b"KJRMAP01";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResidualIdentity {
    pub camera_key: u64,
    /// Explicit seam correction; `[0; 5]` means factory calibration.
    pub calibration: [f64; 5],
    pub pts_ns: u64,
    pub camera: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResidualSidecar {
    pub identity: ResidualIdentity,
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
}

impl ResidualSidecar {
    pub fn read(mut from: impl Read) -> io::Result<Self> {
        let mut magic = [0; 8];
        from.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a KJRMAP01 residual sidecar",
            ));
        }
        let camera_key = u64::from_le_bytes(read(&mut from)?);
        let mut calibration = [0.0; 5];
        for value in &mut calibration {
            *value = f64::from_le_bytes(read(&mut from)?);
        }
        let pts_ns = u64::from_le_bytes(read(&mut from)?);
        let mut camera = [0.0; 3];
        for value in &mut camera {
            *value = f32::from_le_bytes(read(&mut from)?);
        }
        let width = u32::from_le_bytes(read(&mut from)?);
        let height = u32::from_le_bytes(read(&mut from)?);
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "sidecar dimensions overflow")
            })?;
        if count == 0 || count > 1_048_576 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sidecar dimensions unsupported",
            ));
        }
        let mut texels = Vec::with_capacity(count);
        for _ in 0..count {
            let mut texel = [0.0; 4];
            for value in &mut texel {
                *value = f32::from_le_bytes(read(&mut from)?);
            }
            texels.push(texel);
        }
        if !calibration.iter().all(|v| v.is_finite())
            || !camera.iter().all(|v| v.is_finite())
            || !texels.iter().flatten().all(|v| v.is_finite())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sidecar contains non-finite values",
            ));
        }
        Ok(Self {
            identity: ResidualIdentity {
                camera_key,
                calibration,
                pts_ns,
                camera,
            },
            width,
            height,
            texels,
        })
    }
}

fn read<const N: usize>(from: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    from.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::ResidualSidecar;
    #[test]
    fn wrong_magic_refuses_before_any_payload_is_trusted() {
        assert!(ResidualSidecar::read(&b"not-a-map"[..]).is_err());
    }
    #[test]
    fn truncated_header_refuses() {
        assert!(ResidualSidecar::read(&b"KJRMAP01"[..]).is_err());
    }
}
