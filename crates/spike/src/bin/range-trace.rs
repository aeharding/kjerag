//! Authenticated output-view seam traces for a sealed playback range.
//!
//! This consumes saved production maps. It does not run or reconstruct the
//! causal `FrameOwner`, and it paints only the shared actual-alpha crossing
//! law over each receipt-authenticated Kjerag PNG.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::studio_type2::{ALPHA_BYTES, AlphaMap, PACKED_BYTES, PackedMap};
use kjerag_render::{
    Camera, Cue, Horizon, OneXsMapFrame, PisBackend, Sampling, Scene, ScenePipeline, Size,
};
use kjerag_spike::{Gpu, Seam, seam_trace::trace_alpha};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "kjerag.playback-consecutive-range.v2";
const CLAIM: &str = "exact consecutive displayed production frames from one causal frame-zero run";
const OUTPUT_SCHEMA: &str = "kjerag.playback-output-seam-trace.v2";
const OUTPUT_CLAIM: &str =
    "actual selected-alpha 0.5 four-neighbour crossings over authenticated Kjerag output PNGs";
const OUTPUT_RECEIPT: &str = "range-trace-receipt.json";
const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BUILD_GIT_HEAD: &str = env!("KJERAG_BUILD_GIT_HEAD");
const BUILD_GIT_TREE: &str = env!("KJERAG_BUILD_GIT_TREE");
const BUILD_GIT_DIRTY: &str = env!("KJERAG_BUILD_GIT_DIRTY");

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let provenance = Provenance::authenticate()?;
    let receipt = AuthenticatedReceipt::open(&options.receipt, &options.receipt_sha256)?;
    let selected = options.frames.select(&receipt.record.frames)?;
    let sources = AuthenticatedSources::open(&receipt.record.source)?;
    let leaves = AuthenticatedFrames::open(receipt.directory(), &receipt.record.frames)?;
    let mut output = Output::begin(&options.out)?;
    let aliases = sources.descriptor_paths()?;
    let gpu = Gpu::open()?;

    for &position in &selected {
        let frame = &receipt.record.frames[position];
        let authenticated = &leaves.0[position];
        authenticated.verify()?;
        let packed_bytes = authenticated.packed.bytes()?;
        let alpha_bytes = authenticated.alpha.bytes()?;
        let packed = decode_packed(&packed_bytes)?;
        let alpha = decode_alpha(&alpha_bytes)?;
        let base = decode_png(
            &authenticated.image.bytes()?,
            frame.image.width,
            frame.image.height,
        )?;
        let timestamp = frame.timestamp()?;

        let scene = Scene::still_pair(&aliases[0], &aliases[1], Cue::Index(frame.index))?;
        let decoded_sources = scene
            .source_paths()
            .ok_or("decoded range frame lost its source paths")?;
        if decoded_sources.as_ref() != aliases.as_slice() {
            return Err(format!(
                "frame {} decoded from source paths in a different order",
                frame.index
            )
            .into());
        }
        let decoded = scene
            .frame()
            .ok_or("decoded range scene did not retain its frame")?;
        if decoded != (frame.index, timestamp) {
            return Err(format!(
                "frame {} decoded as frame {} at {:.9} s",
                frame.index,
                decoded.0,
                decoded.1.as_secs_f64()
            )
            .into());
        }
        let stamp = scene
            .frame_stamp()
            .ok_or("decoded range scene did not retain its opaque frame stamp")?;
        if (stamp.index(), stamp.timestamp()) != (frame.index, timestamp) {
            return Err(format!("frame {} opaque frame binding changed", frame.index).into());
        }
        let backend = match frame.production_map.pis_backend.as_str() {
            "gpu" => PisBackend::Gpu,
            "cpu" => PisBackend::Cpu,
            _ => {
                return Err(format!("frame {} names an unknown PIS backend", frame.index).into());
            }
        };
        let map = OneXsMapFrame::new(stamp, packed, alpha, backend);
        if map.packed().bytes() != packed_bytes || map.alpha().bytes() != alpha_bytes {
            return Err(format!(
                "frame {} authenticated map bytes changed while decoding",
                frame.index
            )
            .into());
        }
        sources.verify()?;
        authenticated.verify()?;
        Seam::Factory.hold(&scene);
        scene.set_horizon(Horizon::Locked);
        scene.set_sampling(Sampling::Sharp);
        scene.set_flow(false);
        let camera = receipt.record.view.camera()?;
        let mut pipeline = ScenePipeline::new(&gpu.device, &gpu.queue, FORMAT);
        pipeline.hold_band(false);
        pipeline.hold_tone(false);
        let prepared = pipeline
            .prepare_one_xs_picture(&scene.primitive(camera), WIDTH as f32 / HEIGHT as f32)
            .ok_or_else(|| format!("frame {} did not prepare its decoded picture", frame.index))?;
        if !prepared.uses_one_xs_type2_projection() {
            return Err(format!(
                "frame {} did not prepare the selected ONE X2 type-2 projection",
                frame.index
            )
            .into());
        }
        if (prepared.frame().index(), prepared.frame().timestamp()) != (frame.index, timestamp) {
            return Err(format!("frame {} prepared-picture binding changed", frame.index).into());
        }
        let raster = map.rasterize(&prepared, Size::new(WIDTH, HEIGHT))?;
        if raster.uncovered() != 0 {
            return Err(format!(
                "frame {} type-2 raster has {} uncovered output pixels",
                frame.index,
                raster.uncovered()
            )
            .into());
        }
        let traced = trace_alpha(&base, raster.dense())?;
        let png = encode_png(&traced, WIDTH, HEIGHT)?;
        sources.verify()?;
        authenticated.verify()?;
        output.write_frame(frame, &png)?;
        println!(
            "trace: frame {} at {:.9} s, zero uncovered pixels",
            frame.index,
            timestamp.as_secs_f64()
        );
    }

    sources.verify()?;
    leaves.verify_selected(&selected)?;
    receipt.verify()?;
    output.publish(&receipt, &sources, &selected, &provenance)?;
    println!("receipt: {}", options.out.join(OUTPUT_RECEIPT).display());
    println!("boundary: saved production maps were consumed; FrameOwner was not run");
    Ok(())
}

struct Options {
    receipt: PathBuf,
    receipt_sha256: String,
    out: PathBuf,
    frames: FrameSelection,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut receipt = None;
        let mut receipt_sha256 = None;
        let mut out = None;
        let mut frames = None;
        for argument in args {
            if let Some(value) = argument.strip_prefix("--receipt-sha256=") {
                if receipt_sha256.replace(value.to_owned()).is_some() {
                    return Err("--receipt-sha256 was supplied more than once".into());
                }
            } else if let Some(value) = argument.strip_prefix("out-dir=") {
                if out.replace(PathBuf::from(value)).is_some() {
                    return Err("out-dir was supplied more than once".into());
                }
            } else if let Some(value) = argument.strip_prefix("frames=") {
                if frames.replace(FrameSelection::parse(value)?).is_some() {
                    return Err("frames was supplied more than once".into());
                }
            } else if argument.contains('=')
                || receipt.replace(PathBuf::from(argument.clone())).is_some()
            {
                return Err(format!("unexpected or duplicate argument {argument:?}").into());
            }
        }
        let receipt = receipt.ok_or(
            "usage: range-trace RECEIPT --receipt-sha256=HEX out-dir=NEW frames=all|INDEX,...",
        )?;
        let receipt_sha256 = receipt_sha256.ok_or("--receipt-sha256 is required")?;
        require_lower_hex(&receipt_sha256, 64, "receipt SHA-256")?;
        let out = out.ok_or("out-dir is required")?;
        let frames = frames.ok_or("frames is required")?;
        Ok(Self {
            receipt,
            receipt_sha256,
            out,
            frames,
        })
    }
}

enum FrameSelection {
    All,
    List(Vec<u64>),
}

impl FrameSelection {
    fn parse(value: &str) -> Fallible<Self> {
        if value == "all" {
            return Ok(Self::All);
        }
        if value.is_empty() {
            return Err("frames list is empty".into());
        }
        let mut list = Vec::new();
        let mut seen = BTreeSet::new();
        for item in value.split(',') {
            if item.is_empty() || !item.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!("invalid frame index {item:?}").into());
            }
            let index = item.parse::<u64>()?;
            if !seen.insert(index) {
                return Err(format!("frame {index} is selected more than once").into());
            }
            list.push(index);
        }
        Ok(Self::List(list))
    }

    fn select(&self, frames: &[FrameRecord]) -> Fallible<Vec<usize>> {
        match self {
            Self::All => Ok((0..frames.len()).collect()),
            Self::List(list) => list
                .iter()
                .map(|wanted| {
                    frames
                        .iter()
                        .position(|frame| frame.index == *wanted)
                        .ok_or_else(|| {
                            format!("selected frame {wanted} is not in the receipt").into()
                        })
                })
                .collect(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    claim: String,
    limitations: Limitations,
    request: Request,
    view: View,
    source: Vec<SourceRecord>,
    build: Build,
    frames: Vec<FrameRecord>,
    run: Run,
}

impl Receipt {
    fn validate(&self) -> Fallible<()> {
        if self.schema != SCHEMA || self.claim != CLAIM {
            return Err(
                "range receipt schema or claim does not match the producer contract".into(),
            );
        }
        self.limitations.validate()?;
        self.request.validate()?;
        self.view.validate()?;
        if self.source.len() != 2 {
            return Err("range receipt must name exactly two decoder sources".into());
        }
        for (lane, source) in self.source.iter().enumerate() {
            source.validate(lane)?;
        }
        if self.source[0].path == self.source[1].path {
            return Err("range receipt names the same canonical source path twice".into());
        }
        if self.source[0].stable_identity.device == self.source[1].stable_identity.device
            && self.source[0].stable_identity.inode == self.source[1].stable_identity.inode
        {
            return Err("range receipt names the same historical source file twice".into());
        }
        if self.source.iter().filter(|source| source.picked).count() != 1 || !self.source[0].picked
        {
            return Err("range receipt must mark only decoder lane zero as picked".into());
        }
        self.build.validate()?;
        if self.frames.len() as u64 != self.request.count || self.frames.is_empty() {
            return Err("range receipt frame count does not match its request".into());
        }
        let mut files = BTreeSet::new();
        for (offset, frame) in self.frames.iter().enumerate() {
            let expected = self
                .request
                .start
                .checked_add(offset as u64)
                .ok_or("range frame index overflow")?;
            frame.validate(expected, &self.view, &mut files)?;
            if offset > 0 && frame.timestamp()? <= self.frames[offset - 1].timestamp()? {
                return Err("range receipt frame timestamps are not strictly increasing".into());
            }
        }
        if self.frames.last().map(|frame| frame.index) != Some(self.request.end_inclusive) {
            return Err("range receipt last frame does not match end_inclusive".into());
        }
        self.run.validate(&self.request)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Limitations {
    studio_parity_claimed: bool,
    temporal_parity_claimed: bool,
    dense_seam_trace_included: bool,
}

impl Limitations {
    fn validate(&self) -> Fallible<()> {
        if self.studio_parity_claimed
            || self.temporal_parity_claimed
            || self.dense_seam_trace_included
        {
            return Err("range receipt limitations make an unsupported claim".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    start: u64,
    count: u64,
    end_inclusive: u64,
    no_seek: bool,
    cold_start_at_range_boundary: bool,
    every_source_frame_consumed: bool,
    captured_map_substitution: bool,
}

impl Request {
    fn validate(&self) -> Fallible<()> {
        if self.count == 0
            || self.start.checked_add(self.count - 1) != Some(self.end_inclusive)
            || !self.no_seek
            || self.cold_start_at_range_boundary
            || !self.every_source_frame_consumed
            || self.captured_map_substitution
        {
            return Err(
                "range receipt request does not describe one exact causal consecutive run".into(),
            );
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct View {
    yaw_radians: f64,
    pitch_radians: f64,
    fov_radians: f64,
    yaw_degrees: f64,
    pitch_degrees: f64,
    fov_degrees: f64,
    horizon_locked: bool,
    readout: String,
    sampling: String,
    seam_band: bool,
    exposure_tone: bool,
    render_format: String,
    render_width: u32,
    render_height: u32,
    capture_width: u32,
    capture_height: u32,
}

impl View {
    fn validate(&self) -> Fallible<()> {
        let values = [
            self.yaw_radians,
            self.pitch_radians,
            self.fov_radians,
            self.yaw_degrees,
            self.pitch_degrees,
            self.fov_degrees,
        ];
        if !values.into_iter().all(f64::is_finite)
            || (self.yaw_radians as f32).to_degrees().to_bits()
                != (self.yaw_degrees as f32).to_bits()
            || (self.pitch_radians as f32).to_degrees().to_bits()
                != (self.pitch_degrees as f32).to_bits()
            || (self.fov_radians as f32).to_degrees().to_bits()
                != (self.fov_degrees as f32).to_bits()
            || !self.horizon_locked
            || self.readout != "file"
            || self.sampling != "Sharp"
            || !self.seam_band
            || !self.exposure_tone
            || self.render_format != "rgba8unorm"
            || self.render_width != 2560
            || self.render_height != 1440
            || self.capture_width != WIDTH
            || self.capture_height != HEIGHT
        {
            return Err(
                "range receipt view does not match the authenticated output-view contract".into(),
            );
        }
        Ok(())
    }

    fn camera(&self) -> Fallible<Camera> {
        self.validate()?;
        Ok(Camera {
            yaw: self.yaw_radians as f32,
            pitch: self.pitch_radians as f32,
            fov: self.fov_radians as f32,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceRecord {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    stable_identity: HistoricalIdentity,
    decoder_lane: usize,
    picked: bool,
}

impl SourceRecord {
    fn validate(&self, lane: usize) -> Fallible<()> {
        require_exact_canonical_path(&self.path, "source path")?;
        require_lower_hex(&self.sha256, 64, "source SHA-256")?;
        self.stable_identity.validate("source identity")?;
        if self.bytes == 0 || self.decoder_lane != lane {
            return Err(format!("range source position {lane} has an invalid lane or size").into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct HistoricalIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    uid: u32,
    mtime_seconds: i64,
    mtime_nanoseconds: i64,
    ctime_seconds: i64,
    ctime_nanoseconds: i64,
}

impl HistoricalIdentity {
    fn validate(&self, label: &str) -> Fallible<()> {
        if self.links == 0
            || !(0..1_000_000_000).contains(&self.mtime_nanoseconds)
            || !(0..1_000_000_000).contains(&self.ctime_nanoseconds)
        {
            return Err(format!("{label} has invalid filesystem fields").into());
        }
        #[cfg(unix)]
        if self.mode & libc::S_IFMT != libc::S_IFREG {
            return Err(format!("{label} does not describe a regular file").into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Build {
    package_version: String,
    embedded_git_commit: String,
    embedded_git_tree: String,
    embedded_git_dirty: String,
    dirty_at_build: bool,
    runtime_git_commit: String,
    runtime_git_tree: String,
    runtime_tracked_tree_clean: bool,
    executable: IdentifiedFile,
}

impl Build {
    fn validate(&self) -> Fallible<()> {
        require_lower_hex(&self.embedded_git_commit, 40, "embedded git commit")?;
        require_lower_hex(&self.embedded_git_tree, 40, "embedded git tree")?;
        require_lower_hex(&self.runtime_git_commit, 40, "runtime git commit")?;
        require_lower_hex(&self.runtime_git_tree, 40, "runtime git tree")?;
        if self.package_version.is_empty()
            || self.embedded_git_dirty != "false"
            || self.dirty_at_build
            || !self.runtime_tracked_tree_clean
            || self.embedded_git_commit != self.runtime_git_commit
            || self.embedded_git_tree != self.runtime_git_tree
        {
            return Err("range receipt build is not one matching clean build and runtime".into());
        }
        self.executable.validate("historical executable")?;
        if self.executable.file != Path::new("/proc/self/exe") {
            return Err("range receipt names an unexpected historical executable".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IdentifiedFile {
    file: PathBuf,
    bytes: u64,
    sha256: String,
    stable_identity: HistoricalIdentity,
}

impl IdentifiedFile {
    fn validate(&self, label: &str) -> Fallible<()> {
        if self.bytes == 0 {
            return Err(format!("{label} is empty").into());
        }
        require_lower_hex(&self.sha256, 64, &format!("{label} SHA-256"))?;
        self.stable_identity.validate(label)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FrameRecord {
    index: u64,
    timestamp_seconds: u64,
    timestamp_nanoseconds: u32,
    image: ImageRecord,
    production_map: ProductionMap,
}

impl FrameRecord {
    fn timestamp(&self) -> Fallible<Duration> {
        if self.timestamp_nanoseconds >= 1_000_000_000 {
            return Err(format!("frame {} has invalid timestamp nanoseconds", self.index).into());
        }
        Ok(Duration::new(
            self.timestamp_seconds,
            self.timestamp_nanoseconds,
        ))
    }

    fn validate(&self, expected: u64, view: &View, files: &mut BTreeSet<PathBuf>) -> Fallible<()> {
        if self.index != expected {
            return Err(format!("range receipt has a gap or duplicate at frame {expected}").into());
        }
        self.timestamp()?;
        let stem = format!("frame-{expected:010}");
        self.image.validate(&format!("{stem}.png"), view)?;
        self.production_map.packed.validate(
            &format!("{stem}.packed-f32le.bin"),
            PACKED_BYTES,
            "packed map",
        )?;
        self.production_map.alpha.validate(
            &format!("{stem}.alpha-f32le.bin"),
            ALPHA_BYTES,
            "alpha map",
        )?;
        if self.production_map.pis_backend != "gpu" {
            return Err(
                format!("range frame {expected} production map did not use GPU PIS").into(),
            );
        }
        for file in [
            &self.image.file,
            &self.production_map.packed.file,
            &self.production_map.alpha.file,
        ] {
            if !files.insert(file.clone()) {
                return Err(format!("range receipt reuses leaf {}", file.display()).into());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImageRecord {
    file: PathBuf,
    width: u32,
    height: u32,
    bytes: u64,
    sha256: String,
}

impl ImageRecord {
    fn validate(&self, expected_file: &str, view: &View) -> Fallible<()> {
        require_leaf(&self.file, "image filename")?;
        if self.file != Path::new(expected_file)
            || self.width != view.capture_width
            || self.height != view.capture_height
            || self.bytes == 0
        {
            return Err("range image record does not match its exact frame and view".into());
        }
        require_lower_hex(&self.sha256, 64, "image SHA-256")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProductionMap {
    pis_backend: String,
    packed: LeafRecord,
    alpha: LeafRecord,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LeafRecord {
    file: PathBuf,
    bytes: u64,
    sha256: String,
}

impl LeafRecord {
    fn validate(&self, expected_file: &str, bytes: usize, label: &str) -> Fallible<()> {
        require_leaf(&self.file, &format!("{label} filename"))?;
        if self.file != Path::new(expected_file) || self.bytes != bytes as u64 {
            return Err(format!("range {label} record has the wrong filename or size").into());
        }
        require_lower_hex(&self.sha256, 64, &format!("{label} SHA-256"))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Run {
    presented: u64,
    dropped: u64,
    starved: u64,
    scene_redraws: u64,
    instrument_redraws: u64,
    gpu_pis_transactions: u64,
    cpu_pis_transactions: u64,
    elapsed_seconds: u64,
    elapsed_nanoseconds: u32,
}

impl Run {
    fn validate(&self, request: &Request) -> Fallible<()> {
        let expected_presented = request
            .end_inclusive
            .checked_add(1)
            .ok_or("range receipt presentation count overflows")?;
        if self.presented != expected_presented
            || self.dropped != 0
            || self.starved != 0
            // `presented` counts only a newly claimed source pair. The
            // player's redraw count also includes a pump which finds no new
            // pair, including the unanchored startup pump before frame zero.
            // Selected playback can additionally hold that offered pair for
            // a render redraw while its exact map transaction completes; that
            // gate returns before `Player::pump`, so only the instrument count
            // advances. Preserve the causal ordering instead of requiring the
            // three distinct counters to be identical.
            || self.scene_redraws < self.presented
            || self.instrument_redraws < self.scene_redraws
            || self.gpu_pis_transactions != self.presented
            || self.cpu_pis_transactions != 0
            || self.elapsed_nanoseconds >= 1_000_000_000
            || (self.elapsed_seconds == 0 && self.elapsed_nanoseconds == 0)
        {
            return Err(
                "range receipt run statistics do not authenticate the exact request".into(),
            );
        }
        Ok(())
    }
}

struct AuthenticatedReceipt {
    path: PathBuf,
    identity: StableIdentity,
    sha256: String,
    bytes: u64,
    file: File,
    record: Receipt,
}

impl AuthenticatedReceipt {
    fn open(path: &Path, expected_sha256: &str) -> Fallible<Self> {
        require_exact_canonical_path(path, "range receipt")?;
        let retained = RetainedFile::open(path, None, expected_sha256, "range receipt")?;
        let bytes = retained.bytes()?;
        let record: Receipt = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        record.validate()?;
        Ok(Self {
            path: retained.path,
            identity: retained.identity,
            sha256: retained.sha256,
            bytes: retained.bytes,
            file: retained.file,
            record,
        })
    }

    fn directory(&self) -> &Path {
        self.path.parent().expect("canonical receipt has a parent")
    }

    fn verify(&self) -> Fallible<()> {
        RetainedFile::verify_parts(
            &self.path,
            &self.file,
            &self.identity,
            self.bytes,
            &self.sha256,
            "range receipt",
        )
    }
}

struct AuthenticatedSource {
    retained: RetainedFile,
}

struct AuthenticatedSources([AuthenticatedSource; 2]);

impl AuthenticatedSources {
    fn open(records: &[SourceRecord]) -> Fallible<Self> {
        let open = |record: &SourceRecord| -> Fallible<AuthenticatedSource> {
            let retained =
                RetainedFile::open(&record.path, Some(record.bytes), &record.sha256, "source")?;
            if retained.identity.historical()? != record.stable_identity {
                return Err(format!(
                    "{} current identity differs from the receipt identity",
                    record.path.display()
                )
                .into());
            }
            Ok(AuthenticatedSource { retained })
        };
        let sources = [open(&records[0])?, open(&records[1])?];
        if sources[0]
            .retained
            .identity
            .same_file(&sources[1].retained.identity)
        {
            return Err("range sources opened as the same retained file identity".into());
        }
        Ok(Self(sources))
    }

    fn descriptor_paths(&self) -> Fallible<[PathBuf; 2]> {
        Ok([
            descriptor_path(&self.0[0].retained)?,
            descriptor_path(&self.0[1].retained)?,
        ])
    }

    fn verify(&self) -> Fallible<()> {
        self.0
            .iter()
            .try_for_each(|source| source.retained.verify())
    }
}

struct AuthenticatedFrame {
    image: RetainedFile,
    packed: RetainedFile,
    alpha: RetainedFile,
}

impl AuthenticatedFrame {
    fn verify(&self) -> Fallible<()> {
        self.image.verify()?;
        self.packed.verify()?;
        self.alpha.verify()
    }
}

struct AuthenticatedFrames(Vec<AuthenticatedFrame>);

impl AuthenticatedFrames {
    fn open(directory: &Path, frames: &[FrameRecord]) -> Fallible<Self> {
        let mut result = Vec::with_capacity(frames.len());
        for frame in frames {
            result.push(AuthenticatedFrame {
                image: RetainedFile::open(
                    &directory.join(&frame.image.file),
                    Some(frame.image.bytes),
                    &frame.image.sha256,
                    "range image",
                )?,
                packed: RetainedFile::open(
                    &directory.join(&frame.production_map.packed.file),
                    Some(frame.production_map.packed.bytes),
                    &frame.production_map.packed.sha256,
                    "range packed map",
                )?,
                alpha: RetainedFile::open(
                    &directory.join(&frame.production_map.alpha.file),
                    Some(frame.production_map.alpha.bytes),
                    &frame.production_map.alpha.sha256,
                    "range alpha map",
                )?,
            });
        }
        Ok(Self(result))
    }

    fn verify_selected(&self, selected: &[usize]) -> Fallible<()> {
        selected
            .iter()
            .try_for_each(|position| self.0[*position].verify())
    }
}

struct RetainedFile {
    path: PathBuf,
    identity: StableIdentity,
    bytes: u64,
    sha256: String,
    file: File,
}

impl RetainedFile {
    fn open(
        path: &Path,
        expected_bytes: Option<u64>,
        expected_sha256: &str,
        label: &str,
    ) -> Fallible<Self> {
        require_lower_hex(expected_sha256, 64, &format!("{label} SHA-256"))?;
        let before =
            fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(format!("{label} {} is not a regular file", path.display()).into());
        }
        if expected_bytes.is_some_and(|bytes| before.len() != bytes) {
            return Err(format!("{label} {} has the wrong byte size", path.display()).into());
        }
        let identity = stable_identity(&before);
        let mut file = open_nofollow(path)?;
        if stable_identity(&file.metadata()?) != identity {
            return Err(format!("{label} {} changed while opening", path.display()).into());
        }
        let sha256 = sha256_reader(BufReader::new(&mut file))?;
        let named = fs::symlink_metadata(path)?;
        if stable_identity(&file.metadata()?) != identity || stable_identity(&named) != identity {
            return Err(format!("{label} {} changed while hashing", path.display()).into());
        }
        if sha256 != expected_sha256 {
            return Err(format!(
                "{label} {} has SHA-256 {sha256}, expected {expected_sha256}",
                path.display()
            )
            .into());
        }
        Ok(Self {
            path: path.to_owned(),
            identity,
            bytes: before.len(),
            sha256,
            file,
        })
    }

    fn bytes(&self) -> Fallible<Vec<u8>> {
        self.verify()?;
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::with_capacity(self.bytes.try_into().unwrap_or(0));
        file.read_to_end(&mut bytes)?;
        if stable_identity(&file.metadata()?) != self.identity || bytes.len() as u64 != self.bytes {
            return Err(format!("{} changed while loading", self.path.display()).into());
        }
        Ok(bytes)
    }

    fn verify(&self) -> Fallible<()> {
        Self::verify_parts(
            &self.path,
            &self.file,
            &self.identity,
            self.bytes,
            &self.sha256,
            "authenticated leaf",
        )
    }

    fn verify_parts(
        path: &Path,
        file: &File,
        identity: &StableIdentity,
        bytes: u64,
        sha256: &str,
        label: &str,
    ) -> Fallible<()> {
        let named = fs::symlink_metadata(path)
            .map_err(|error| format!("{} changed: {error}", path.display()))?;
        if stable_identity(&named) != *identity || stable_identity(&file.metadata()?) != *identity {
            return Err(format!("{label} {} identity changed", path.display()).into());
        }
        let mut clone = file.try_clone()?;
        clone.seek(SeekFrom::Start(0))?;
        let actual = sha256_reader(BufReader::new(&mut clone))?;
        if stable_identity(&clone.metadata()?) != *identity
            || identity.len != bytes
            || actual != sha256
        {
            return Err(format!("{label} {} bytes changed", path.display()).into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StableIdentity {
    len: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    links: u64,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    mtime_seconds: i64,
    #[cfg(unix)]
    mtime_nanoseconds: i64,
    #[cfg(unix)]
    ctime_seconds: i64,
    #[cfg(unix)]
    ctime_nanoseconds: i64,
}

impl StableIdentity {
    fn historical(&self) -> Fallible<HistoricalIdentity> {
        #[cfg(unix)]
        return Ok(HistoricalIdentity {
            device: self.device,
            inode: self.inode,
            mode: self.mode,
            links: self.links,
            uid: self.uid,
            mtime_seconds: self.mtime_seconds,
            mtime_nanoseconds: self.mtime_nanoseconds,
            ctime_seconds: self.ctime_seconds,
            ctime_nanoseconds: self.ctime_nanoseconds,
        });
        #[cfg(not(unix))]
        Err("range source identity authentication requires Unix metadata".into())
    }

    fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        return self.device == other.device && self.inode == other.inode;
        #[cfg(not(unix))]
        false
    }
}

fn stable_identity(metadata: &fs::Metadata) -> StableIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        StableIdentity {
            len: metadata.len(),
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            uid: metadata.uid(),
            mtime_seconds: metadata.mtime(),
            mtime_nanoseconds: metadata.mtime_nsec(),
            ctime_seconds: metadata.ctime(),
            ctime_nanoseconds: metadata.ctime_nsec(),
        }
    }
    #[cfg(not(unix))]
    StableIdentity {
        len: metadata.len(),
    }
}

#[cfg(target_os = "linux")]
fn descriptor_path(source: &RetainedFile) -> Fallible<PathBuf> {
    use std::os::fd::AsRawFd as _;
    source.verify()?;
    let alias = PathBuf::from(format!("/proc/self/fd/{}", source.file.as_raw_fd()));
    if stable_identity(&fs::metadata(&alias)?) != source.identity {
        return Err(format!("{} descriptor alias changed", source.path.display()).into());
    }
    Ok(alias)
}

#[cfg(not(target_os = "linux"))]
fn descriptor_path(_source: &RetainedFile) -> Fallible<PathBuf> {
    Err("range trace decode requires Linux /proc/self/fd".into())
}

fn decode_packed(bytes: &[u8]) -> Fallible<PackedMap> {
    if bytes.len() != PACKED_BYTES {
        return Err("packed map has the wrong byte size".into());
    }
    let nodes = bytes
        .chunks_exact(16)
        .map(|node| {
            [
                f32::from_le_bytes(node[0..4].try_into().expect("four bytes")),
                f32::from_le_bytes(node[4..8].try_into().expect("four bytes")),
                f32::from_le_bytes(node[8..12].try_into().expect("four bytes")),
                f32::from_le_bytes(node[12..16].try_into().expect("four bytes")),
            ]
        })
        .collect();
    Ok(PackedMap::new(nodes)?)
}

fn decode_alpha(bytes: &[u8]) -> Fallible<AlphaMap> {
    if bytes.len() != ALPHA_BYTES {
        return Err("alpha map has the wrong byte size".into());
    }
    let nodes = bytes
        .chunks_exact(4)
        .map(|node| f32::from_le_bytes(node.try_into().expect("four bytes")))
        .collect();
    Ok(AlphaMap::new(nodes)?)
}

struct Provenance {
    workspace: PathBuf,
    runtime_head: String,
    runtime_tree: String,
    executable: RetainedExecutable,
    executable_identity: Value,
}

impl Provenance {
    fn authenticate() -> Fallible<Self> {
        let workspace = workspace_path()?;
        let (runtime_head, runtime_tree) = checkout_state(&workspace)?;
        validate_build_provenance(&runtime_head, &runtime_tree)?;
        let executable = RetainedExecutable::open()?;
        let executable_identity = executable.identity()?;
        Ok(Self {
            workspace,
            runtime_head,
            runtime_tree,
            executable,
            executable_identity,
        })
    }

    fn receipt(&self) -> Value {
        json!({
            "package_version": env!("CARGO_PKG_VERSION"),
            "embedded_git_commit": BUILD_GIT_HEAD,
            "embedded_git_tree": BUILD_GIT_TREE,
            "embedded_git_dirty": BUILD_GIT_DIRTY,
            "dirty_at_build": false,
            "runtime_git_commit": self.runtime_head,
            "runtime_git_tree": self.runtime_tree,
            "runtime_tracked_tree_clean": true,
            "executable": self.executable_identity
        })
    }

    fn verify(&self) -> Fallible<()> {
        if self.executable.identity()? != self.executable_identity {
            return Err("range-trace executable changed during the run".into());
        }
        let (head, tree) = checkout_state(&self.workspace)?;
        validate_build_provenance(&head, &tree)?;
        if head != self.runtime_head || tree != self.runtime_tree {
            return Err("range-trace runtime checkout changed during the run".into());
        }
        Ok(())
    }
}

struct RetainedExecutable {
    file: File,
    identity: StableIdentity,
    sha256: String,
}

impl RetainedExecutable {
    #[cfg(target_os = "linux")]
    fn open() -> Fallible<Self> {
        let mut file = File::open("/proc/self/exe")?;
        let identity = stable_identity(&file.metadata()?);
        let sha256 = sha256_reader(BufReader::new(&mut file))?;
        if stable_identity(&file.metadata()?) != identity {
            return Err("range-trace executable changed while hashing".into());
        }
        Ok(Self {
            file,
            identity,
            sha256,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn open() -> Fallible<Self> {
        Err("range-trace executable authentication requires Linux /proc/self/exe".into())
    }

    fn identity(&self) -> Fallible<Value> {
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let sha256 = sha256_reader(BufReader::new(&mut file))?;
        if stable_identity(&file.metadata()?) != self.identity || sha256 != self.sha256 {
            return Err("range-trace executable changed during verification".into());
        }
        Ok(json!({
            "file": "/proc/self/exe",
            "bytes": self.identity.len,
            "sha256": self.sha256,
            "stable_identity": self.identity.historical()?
        }))
    }
}

fn workspace_path() -> Fallible<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("spike manifest is not inside the workspace")?
        .to_owned())
}

fn checkout_state(workspace: &Path) -> Fallible<(String, String)> {
    let head = git_line(workspace, &["rev-parse", "--verify", "HEAD"])?;
    let tree = git_line(workspace, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    let status = git_line(
        workspace,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )?;
    if !status.is_empty() {
        return Err("range-trace requires a clean tracked runtime worktree".into());
    }
    Ok((head, tree))
}

fn validate_build_provenance(runtime_head: &str, runtime_tree: &str) -> Fallible<()> {
    if BUILD_GIT_DIRTY != "false" {
        return Err(format!(
            "range-trace requires a clean build, but build-time tracked-tree state is {BUILD_GIT_DIRTY}"
        )
        .into());
    }
    if BUILD_GIT_HEAD != runtime_head || BUILD_GIT_TREE != runtime_tree {
        return Err(format!(
            "range-trace build provenance {BUILD_GIT_HEAD}/{BUILD_GIT_TREE} differs from runtime checkout {runtime_head}/{runtime_tree}"
        )
        .into());
    }
    Ok(())
}

fn git_line(workspace: &Path, arguments: &[&str]) -> Fallible<String> {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .arg("-C")
        .arg(workspace)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn decode_png(bytes: &[u8], width: u32, height: u32) -> Fallible<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info()?;
    if reader.info().width != width
        || reader.info().height != height
        || reader.info().color_type != png::ColorType::Rgba
        || reader.info().bit_depth != png::BitDepth::Eight
    {
        return Err("authenticated base PNG is not exact RGBA8 at its receipt extent".into());
    }
    let mut output = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut output)?;
    output.truncate(info.buffer_size());
    if output.len() != width as usize * height as usize * 4 {
        return Err("authenticated base PNG decoded to the wrong byte size".into());
    }
    Ok(output)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Fallible<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(rgba)?;
    }
    Ok(encoded)
}

struct Output {
    out: PathBuf,
    stage: Option<PathBuf>,
    frames: Vec<Value>,
    leaves: Vec<OutputLeaf>,
}

#[derive(Clone, Serialize)]
struct OutputLeaf {
    file: PathBuf,
    bytes: u64,
    sha256: String,
}

impl Output {
    fn begin(out: &Path) -> Fallible<Self> {
        if out.exists() {
            return Err(format!("out-dir {} already exists", out.display()).into());
        }
        let stage = stage_path(out)?;
        if stage.exists() {
            return Err(format!("staging directory {} already exists", stage.display()).into());
        }
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&stage)?;
        Ok(Self {
            out: out.to_owned(),
            stage: Some(stage),
            frames: Vec::new(),
            leaves: Vec::new(),
        })
    }

    fn write_frame(&mut self, frame: &FrameRecord, png: &[u8]) -> Fallible<()> {
        let name = format!("frame-{:010}-trace.png", frame.index);
        let stage = self
            .stage
            .as_deref()
            .ok_or("trace output was already published")?;
        write_new(&stage.join(&name), png)?;
        let leaf = OutputLeaf {
            file: PathBuf::from(&name),
            bytes: png.len() as u64,
            sha256: sha256_bytes(png),
        };
        self.frames.push(json!({
            "index": frame.index,
            "timestamp_seconds": frame.timestamp_seconds,
            "timestamp_nanoseconds": frame.timestamp_nanoseconds,
            "packed_sha256": frame.production_map.packed.sha256,
            "alpha_sha256": frame.production_map.alpha.sha256,
            "base_png_sha256": frame.image.sha256,
            "trace": {
                "file": name,
                "bytes": leaf.bytes,
                "sha256": leaf.sha256
            }
        }));
        self.leaves.push(leaf);
        Ok(())
    }

    fn publish(
        &mut self,
        input: &AuthenticatedReceipt,
        sources: &AuthenticatedSources,
        selected: &[usize],
        provenance: &Provenance,
    ) -> Fallible<()> {
        if self.frames.len() != selected.len() {
            return Err("trace output is incomplete".into());
        }
        let source = input
            .record
            .source
            .iter()
            .zip(&sources.0)
            .map(|(record, authenticated)| -> Fallible<Value> {
                Ok(json!({
                    "decoder_lane": record.decoder_lane,
                    "path": record.path,
                    "bytes": authenticated.retained.bytes,
                    "sha256": authenticated.retained.sha256,
                    "stable_identity": authenticated.retained.identity.historical()?
                }))
            })
            .collect::<Fallible<Vec<_>>>()?;
        let receipt = json!({
            "schema": OUTPUT_SCHEMA,
            "claim": OUTPUT_CLAIM,
            "limitations": {
                "frame_owner_rerun": false,
                "studio_parity_claimed": false,
                "temporal_parity_claimed": false,
                "base_pixels_rerendered": false
            },
            "input_receipt": {
                "path": input.path,
                "bytes": input.bytes,
                "sha256": input.sha256,
                "schema": input.record.schema,
                "claim": input.record.claim
            },
            "request": input.record.request,
            "view": {
                "yaw_radians": input.record.view.yaw_radians,
                "pitch_radians": input.record.view.pitch_radians,
                "fov_radians": input.record.view.fov_radians,
                "yaw_degrees": input.record.view.yaw_degrees,
                "pitch_degrees": input.record.view.pitch_degrees,
                "fov_degrees": input.record.view.fov_degrees,
                "horizon_locked": true,
                "readout": "file",
                "sampling": "Sharp",
                "seam": "factory",
                "seam_band": true,
                "exposure_tone": true,
                "width": WIDTH,
                "height": HEIGHT
            },
            "source": source,
            "build": provenance.receipt(),
            "selected_frames": selected.iter().map(|position| input.record.frames[*position].index).collect::<Vec<_>>(),
            "frames": self.frames,
            "inventory": {
                "trace_leaves": self.leaves,
                "receipt": {
                    "file": OUTPUT_RECEIPT,
                    "self_hash_recorded": false
                },
                "directory_entries": self.leaves.len() + 1
            },
            "input_run": input.record.run
        });
        let mut encoded = serde_json::to_vec_pretty(&receipt)?;
        encoded.push(b'\n');
        let stage = self
            .stage
            .as_deref()
            .ok_or("trace output was already published")?;
        write_new(&stage.join(OUTPUT_RECEIPT), &encoded)?;
        let mut expected = self.leaves.clone();
        expected.push(OutputLeaf {
            file: PathBuf::from(OUTPUT_RECEIPT),
            bytes: encoded.len() as u64,
            sha256: sha256_bytes(&encoded),
        });
        verify_staging(stage, &expected)?;
        provenance.verify()?;
        sync_directory(stage)?;
        rename_noreplace(stage, &self.out)?;
        sync_directory(self.out.parent().unwrap_or(Path::new(".")))?;
        self.stage = None;
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(stage) = &self.stage {
            let _ = fs::remove_dir_all(stage);
        }
    }
}

fn stage_path(out: &Path) -> Fallible<PathBuf> {
    let name = out
        .file_name()
        .ok_or("out-dir must name a directory")?
        .to_string_lossy();
    Ok(out
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{name}.range-trace-tmp-{}", std::process::id())))
}

fn write_new(path: &Path, bytes: &[u8]) -> Fallible<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn verify_staging(stage: &Path, expected: &[OutputLeaf]) -> Fallible<()> {
    let expected_names = expected
        .iter()
        .map(|leaf| leaf.file.clone())
        .collect::<BTreeSet<_>>();
    if expected_names.len() != expected.len() {
        return Err("range-trace output inventory contains a duplicate filename".into());
    }
    let mut actual_names = BTreeSet::new();
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let name = PathBuf::from(entry.file_name());
        require_leaf(&name, "staging entry")?;
        if !actual_names.insert(name) {
            return Err("range-trace staging directory contains a duplicate entry".into());
        }
    }
    if actual_names != expected_names {
        return Err(format!(
            "range-trace staging inventory is not exact: got {actual_names:?}, expected {expected_names:?}"
        )
        .into());
    }
    for leaf in expected {
        verify_written_leaf(&stage.join(&leaf.file), leaf)?;
    }
    Ok(())
}

fn verify_written_leaf(path: &Path, expected: &OutputLeaf) -> Fallible<()> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(format!("staged output {} is not a regular file", path.display()).into());
    }
    let identity = stable_identity(&before);
    if identity.len != expected.bytes {
        return Err(format!("staged output {} has the wrong byte size", path.display()).into());
    }
    let mut file = open_nofollow(path)?;
    if stable_identity(&file.metadata()?) != identity {
        return Err(format!("staged output {} changed while opening", path.display()).into());
    }
    let sha256 = sha256_reader(BufReader::new(&mut file))?;
    let named = fs::symlink_metadata(path)?;
    if stable_identity(&file.metadata()?) != identity || stable_identity(&named) != identity {
        return Err(format!("staged output {} changed while hashing", path.display()).into());
    }
    if sha256 != expected.sha256 {
        return Err(format!(
            "staged output {} has SHA-256 {sha256}, expected {}",
            path.display(),
            expected.sha256
        )
        .into());
    }
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Fallible<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn rename_noreplace(from: &Path, to: &Path) -> Fallible<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_noreplace(_from: &Path, _to: &Path) -> Fallible<()> {
    Err("range trace no-replace publication requires Linux renameat2".into())
}

fn require_exact_canonical_path(path: &Path, label: &str) -> Fallible<()> {
    if !path.is_absolute() || fs::canonicalize(path)?.as_os_str() != path.as_os_str() {
        return Err(format!("{label} must be an exact absolute non-symlink path").into());
    }
    Ok(())
}

fn require_leaf(path: &Path, label: &str) -> Fallible<()> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || path.as_os_str().is_empty()
        || path.to_string_lossy().contains('\\')
    {
        return Err(format!("{label} must be one relative filename").into());
    }
    Ok(())
}

fn open_nofollow(path: &Path) -> Fallible<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn require_lower_hex(value: &str, length: usize, label: &str) -> Fallible<()> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(
            format!("{label} must be exactly {length} lowercase hexadecimal characters").into(),
        );
    }
    Ok(())
}

fn sha256_reader(mut reader: impl Read) -> Fallible<String> {
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(digest_hex(digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    digest_hex(Sha256::digest(bytes))
}

fn digest_hex(digest: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "kjerag-range-trace-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn identity(inode: u64) -> Value {
        json!({
            "device": 1, "inode": inode, "mode": 33188, "links": 1, "uid": 1000,
            "mtime_seconds": 1, "mtime_nanoseconds": 0,
            "ctime_seconds": 1, "ctime_nanoseconds": 0
        })
    }

    fn frame(index: u64, nanoseconds: u32) -> Value {
        let stem = format!("frame-{index:010}");
        json!({
            "index": index,
            "timestamp_seconds": 1,
            "timestamp_nanoseconds": nanoseconds,
            "image": {"file": format!("{stem}.png"), "width": WIDTH, "height": HEIGHT, "bytes": 1, "sha256": "1".repeat(64)},
            "production_map": {
                "pis_backend": "gpu",
                "packed": {"file": format!("{stem}.packed-f32le.bin"), "bytes": PACKED_BYTES, "sha256": "2".repeat(64)},
                "alpha": {"file": format!("{stem}.alpha-f32le.bin"), "bytes": ALPHA_BYTES, "sha256": "3".repeat(64)}
            }
        })
    }

    fn valid(source_a: &Path, source_b: &Path) -> Value {
        json!({
            "schema": SCHEMA,
            "claim": CLAIM,
            "limitations": {"studio_parity_claimed": false, "temporal_parity_claimed": false, "dense_seam_trace_included": false},
            "request": {"start": 7, "count": 2, "end_inclusive": 8, "no_seek": true, "cold_start_at_range_boundary": false, "every_source_frame_consumed": true, "captured_map_substitution": false},
            "view": {"yaw_radians": 1.0_f32, "pitch_radians": -0.25_f32, "fov_radians": 0.75_f32, "yaw_degrees": 1.0_f32.to_degrees(), "pitch_degrees": (-0.25_f32).to_degrees(), "fov_degrees": 0.75_f32.to_degrees(), "horizon_locked": true, "readout": "file", "sampling": "Sharp", "seam_band": true, "exposure_tone": true, "render_format": "rgba8unorm", "render_width": 2560, "render_height": 1440, "capture_width": WIDTH, "capture_height": HEIGHT},
            "source": [
                {"path": source_a, "bytes": 1, "sha256": "4".repeat(64), "stable_identity": identity(2), "decoder_lane": 0, "picked": true},
                {"path": source_b, "bytes": 1, "sha256": "5".repeat(64), "stable_identity": identity(3), "decoder_lane": 1, "picked": false}
            ],
            "build": {"package_version": "0.2.0", "embedded_git_commit": "6".repeat(40), "embedded_git_tree": "7".repeat(40), "embedded_git_dirty": "false", "dirty_at_build": false, "runtime_git_commit": "6".repeat(40), "runtime_git_tree": "7".repeat(40), "runtime_tracked_tree_clean": true, "executable": {"file": "/proc/self/exe", "bytes": 1, "sha256": "8".repeat(64), "stable_identity": identity(4)}},
            "frames": [frame(7, 100), frame(8, 200)],
            "run": {"presented": 9, "dropped": 0, "starved": 0, "scene_redraws": 9, "instrument_redraws": 9, "gpu_pis_transactions": 9, "cpu_pis_transactions": 0, "elapsed_seconds": 1, "elapsed_nanoseconds": 0}
        })
    }

    fn fixture() -> (Temp, Value) {
        let temp = Temp::new();
        let a = temp.0.join("a.insv");
        let b = temp.0.join("b.insv");
        fs::write(&a, [1]).unwrap();
        fs::write(&b, [2]).unwrap();
        (temp, valid(&a, &b))
    }

    #[test]
    fn strict_receipt_rejects_unknown_fields_and_false_claims() {
        let (_temp, value) = fixture();
        let receipt: Receipt = serde_json::from_value(value.clone()).unwrap();
        receipt.validate().unwrap();
        let mut unknown = value.clone();
        unknown["view"]["decoy"] = json!(true);
        assert!(serde_json::from_value::<Receipt>(unknown).is_err());
        let mut false_claim = value;
        false_claim["request"]["no_seek"] = json!(false);
        let receipt: Receipt = serde_json::from_value(false_claim).unwrap();
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn receipt_rejects_cpu_or_incomplete_gpu_pis_provenance() {
        let (_temp, value) = fixture();

        let mut cpu_frame = value.clone();
        cpu_frame["frames"][0]["production_map"]["pis_backend"] = json!("cpu");
        let receipt: Receipt = serde_json::from_value(cpu_frame).unwrap();
        assert!(receipt.validate().is_err());

        let mut missing_gpu = value.clone();
        missing_gpu["run"]["gpu_pis_transactions"] = json!(8);
        let receipt: Receipt = serde_json::from_value(missing_gpu).unwrap();
        assert!(receipt.validate().is_err());

        let mut cpu_count = value;
        cpu_count["run"]["cpu_pis_transactions"] = json!(1);
        let receipt: Receipt = serde_json::from_value(cpu_count).unwrap();
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn historical_v1_without_backend_provenance_cannot_claim_gpu_only_execution() {
        let (_temp, mut value) = fixture();
        value["schema"] = json!("kjerag.playback-consecutive-range.v1");
        value["run"]
            .as_object_mut()
            .unwrap()
            .remove("gpu_pis_transactions");
        value["run"]
            .as_object_mut()
            .unwrap()
            .remove("cpu_pis_transactions");
        for frame in value["frames"].as_array_mut().unwrap() {
            frame["production_map"]
                .as_object_mut()
                .unwrap()
                .remove("pis_backend");
        }
        assert!(serde_json::from_value::<Receipt>(value).is_err());
    }

    #[test]
    fn run_redraws_preserve_the_causal_counter_order() {
        let (_temp, value) = fixture();

        let mut startup_redraw = value.clone();
        startup_redraw["run"]["scene_redraws"] = json!(10);
        startup_redraw["run"]["instrument_redraws"] = json!(10);
        serde_json::from_value::<Receipt>(startup_redraw)
            .unwrap()
            .validate()
            .unwrap();

        let mut selected_map_redraw = value.clone();
        selected_map_redraw["run"]["instrument_redraws"] = json!(10);
        serde_json::from_value::<Receipt>(selected_map_redraw)
            .unwrap()
            .validate()
            .unwrap();

        let mut fewer_scene_redraws_than_presentations = value.clone();
        fewer_scene_redraws_than_presentations["run"]["scene_redraws"] = json!(8);
        assert!(
            serde_json::from_value::<Receipt>(fewer_scene_redraws_than_presentations)
                .unwrap()
                .validate()
                .is_err()
        );

        let mut fewer_instrument_redraws_than_scene_redraws = value;
        fewer_instrument_redraws_than_scene_redraws["run"]["scene_redraws"] = json!(10);
        assert!(
            serde_json::from_value::<Receipt>(fewer_instrument_redraws_than_scene_redraws)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn traversal_gaps_duplicates_and_binding_decoys_are_rejected() {
        let (_temp, value) = fixture();
        let mut traversal = value.clone();
        traversal["frames"][0]["image"]["file"] = json!("../frame.png");
        assert!(
            serde_json::from_value::<Receipt>(traversal)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut gap = value.clone();
        gap["frames"][1] = frame(9, 200);
        assert!(
            serde_json::from_value::<Receipt>(gap)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut duplicate = value.clone();
        duplicate["frames"][1] = frame(7, 200);
        assert!(
            serde_json::from_value::<Receipt>(duplicate)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut view = value;
        view["view"]["sampling"] = json!("Luma");
        assert!(
            serde_json::from_value::<Receipt>(view)
                .unwrap()
                .validate()
                .is_err()
        );

        let (_temp, value) = fixture();
        for (width, height) in [(1920, 1080), (2560, 1080), (1440, 2560)] {
            let mut dimensions = value.clone();
            dimensions["view"]["render_width"] = json!(width);
            dimensions["view"]["render_height"] = json!(height);
            assert!(
                serde_json::from_value::<Receipt>(dimensions)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn duplicate_source_path_and_historical_identity_are_rejected() {
        let (_temp, value) = fixture();
        let mut same_path = value.clone();
        same_path["source"][1]["path"] = same_path["source"][0]["path"].clone();
        assert!(
            serde_json::from_value::<Receipt>(same_path)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut same_identity = value;
        same_identity["source"][1]["stable_identity"] =
            same_identity["source"][0]["stable_identity"].clone();
        assert!(
            serde_json::from_value::<Receipt>(same_identity)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn retained_source_identity_detects_distinct_names_for_one_file() {
        let temp = Temp::new();
        let first = temp.0.join("first.insv");
        let second = temp.0.join("second.insv");
        fs::write(&first, b"source").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let sha256 = sha256_bytes(b"source");
        let first = RetainedFile::open(&first, Some(6), &sha256, "source").unwrap();
        let second = RetainedFile::open(&second, Some(6), &sha256, "source").unwrap();
        assert!(first.identity.same_file(&second.identity));
    }

    #[test]
    fn retained_leaf_detects_mutation() {
        let temp = Temp::new();
        let path = temp.0.join("leaf");
        fs::write(&path, b"first").unwrap();
        let sha = sha256_bytes(b"first");
        let retained = RetainedFile::open(&path, Some(5), &sha, "test leaf").unwrap();
        fs::write(&path, b"other").unwrap();
        assert!(retained.verify().is_err());
    }

    #[test]
    fn output_collision_and_failed_output_cleanup_are_safe() {
        let temp = Temp::new();
        let out = temp.0.join("out");
        fs::create_dir(&out).unwrap();
        assert!(Output::begin(&out).is_err());
        fs::remove_dir(&out).unwrap();
        let stage;
        {
            let output = Output::begin(&out).unwrap();
            stage = output.stage.clone().unwrap();
            assert!(stage.exists());
        }
        assert!(!stage.exists());
        assert!(!out.exists());

        let output = Output::begin(&out).unwrap();
        let stage = output.stage.clone().unwrap();
        fs::create_dir(&out).unwrap();
        assert!(rename_noreplace(&stage, &out).is_err());
        drop(output);
        assert!(!stage.exists());
        assert!(out.exists());
    }

    #[test]
    fn staging_inventory_rejects_mutation_and_extra_leaves() {
        let temp = Temp::new();
        let trace = OutputLeaf {
            file: PathBuf::from("trace.png"),
            bytes: 5,
            sha256: sha256_bytes(b"trace"),
        };
        fs::write(temp.0.join(&trace.file), b"trace").unwrap();
        verify_staging(&temp.0, std::slice::from_ref(&trace)).unwrap();

        fs::write(temp.0.join(&trace.file), b"mutat").unwrap();
        assert!(verify_staging(&temp.0, std::slice::from_ref(&trace)).is_err());
        fs::write(temp.0.join(&trace.file), b"trace").unwrap();
        fs::write(temp.0.join("extra"), b"decoy").unwrap();
        assert!(verify_staging(&temp.0, &[trace]).is_err());
    }

    #[test]
    fn png_trace_round_trip_preserves_base_and_mark_bytes() {
        let mut pixels = (0_u8..64).collect::<Vec<_>>();
        pixels[4 * 5..4 * 6].copy_from_slice(&[255, 40, 40, 255]);
        let encoded = encode_png(&pixels, 4, 4).unwrap();
        let decoded = decode_png(&encoded, 4, 4).unwrap();
        assert_eq!(decoded, pixels);
        assert_eq!(&decoded[4 * 5..4 * 6], &[255, 40, 40, 255]);
        assert_eq!(&decoded[0..4], &[0, 1, 2, 3]);
    }

    #[test]
    fn frame_list_rejects_duplicates_and_missing_receipt_frames() {
        assert!(FrameSelection::parse("7,7").is_err());
        let (_temp, value) = fixture();
        let receipt: Receipt = serde_json::from_value(value).unwrap();
        assert!(
            FrameSelection::parse("9")
                .unwrap()
                .select(&receipt.frames)
                .is_err()
        );
    }
}
