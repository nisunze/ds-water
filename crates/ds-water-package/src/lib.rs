//! A Water network as a governed DS Grid model (ds-cli OPEN §39): the
//! `ds.water_network_snapshot` profile carried by the native `.dsgrid`
//! container, exactly as an LV transformer rides in
//! `ds.lv_transformer_snapshot`.
//!
//! The ordinary typed model tables stay present and empty, so every package
//! passes the standard `.dsgrid` verifier. The network is a closed set of
//! content-addressed assets:
//!
//! - `water-network-v1.json` — the typed profile descriptor;
//! - `water-network-v1.json.zst` — the network (`ds.water.network/v1`) as one
//!   zstd frame of its compact JSON, written by pure-Rust `ruzstd` so native
//!   and WASM hosts write identical bytes for an identical network.
//!
//! The network is admitted through [`WaterNetwork`] before it is packed, so a
//! package never holds a network the engine would refuse to read.

use std::collections::BTreeMap;
use std::io::Read;

use ds_grid_exchange::{AssetBytes, GridPackage, PackOptions, PackageError, pack, unpack};
use ds_grid_model::{EntityId, GridModelSnapshot, ModelCrs, TableKind};
use ds_water_model::WaterNetwork;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROFILE: &str = "ds.water_network_snapshot";
pub const PROFILE_VERSION: u32 = 1;
pub const DESCRIPTOR_LEAF: &str = "water-network-v1.json";
pub const NETWORK_LEAF: &str = "water-network-v1.json.zst";
pub const APPLICATION_CRS: &str = "urn:ds:application-snapshot:water-network-v1";
/// The project grid catalog's kind for a Water network.
pub const MODEL_KIND: &str = "water_network";
/// A decoded network never exceeds this many bytes of JSON.
const MAX_NETWORK_JSON_BYTES: u64 = 256 * 1024 * 1024;

/// One saved Water network.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterNetworkSnapshot {
    /// Origin provenance only, never identity: a network can be designed
    /// without a DS project.
    pub source_project_id: Option<String>,
    /// The catalog model id (`water_<slug>`).
    pub model_id: String,
    /// The saved revision, from 1.
    pub source_revision: u64,
    pub network: WaterNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRef {
    pub leaf: String,
    pub encoding: String,
    pub byte_len: u64,
    pub sha256: String,
    pub junctions: u64,
    pub pipes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub profile: String,
    pub profile_version: u32,
    pub object_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    pub object_id: String,
    pub source_revision: u64,
    pub network: NetworkRef,
}

/// What a packed network declares to the catalog, read back from the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Packed {
    pub manifest_model_id: String,
    pub manifest_model_revision: u64,
    pub model_schema_version: u32,
    /// SHA-256 of the package bytes: the catalog object's digest.
    pub digest: String,
    pub byte_length: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum WaterPackageError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("invalid Water network package: {0}")]
    Invalid(String),
    #[error("Water network package JSON is invalid: {0}")]
    Json(String),
}

fn json(error: serde_json::Error) -> WaterPackageError {
    WaterPackageError::Json(error.to_string())
}

/// Pack one network into its `.dsgrid` package and the identity the catalog
/// declares for it.
pub fn pack_network(
    snapshot: &WaterNetworkSnapshot,
) -> Result<(Vec<u8>, Packed), WaterPackageError> {
    validate_identity(snapshot)?;
    let network_json = serde_json::to_vec(&snapshot.network).map_err(json)?;
    let frame = ruzstd::encoding::compress_to_vec(
        network_json.as_slice(),
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    let descriptor = Descriptor {
        profile: PROFILE.into(),
        profile_version: PROFILE_VERSION,
        object_type: "water_network".into(),
        source_project_id: snapshot.source_project_id.clone(),
        object_id: snapshot.model_id.clone(),
        source_revision: snapshot.source_revision,
        network: NetworkRef {
            leaf: NETWORK_LEAF.into(),
            encoding: "json+zstd".into(),
            byte_len: frame.len() as u64,
            sha256: sha256_hex(&frame),
            junctions: snapshot.network.junctions.len() as u64,
            pipes: snapshot.network.pipes.len() as u64,
        },
    };
    let assets = vec![
        AssetBytes {
            invariant_leaf: NETWORK_LEAF.into(),
            bytes: frame,
        },
        AssetBytes {
            invariant_leaf: DESCRIPTOR_LEAF.into(),
            bytes: serde_json::to_vec_pretty(&descriptor).map_err(json)?,
        },
    ];
    let invalid = |error: String| WaterPackageError::Invalid(error);
    let bytes = pack(
        &GridModelSnapshot::default(),
        &PackOptions {
            presentation: Default::default(),
            model_id: EntityId::new(manifest_model_id(&snapshot.model_id))
                .map_err(|e| invalid(e.to_string()))?,
            model_revision: snapshot.source_revision,
            coordinate_system: ModelCrs::new(APPLICATION_CRS)
                .map_err(|e| invalid(e.to_string()))?,
            library_pins: Vec::new(),
            library_needs: Vec::new(),
            assets,
            exchange_bindings: BTreeMap::new(),
        },
    )?;
    let manifest = unpack(&bytes)?.manifest;
    let packed = Packed {
        manifest_model_id: manifest.model.model_id.to_string(),
        manifest_model_revision: manifest.model.model_revision,
        model_schema_version: manifest.model.schema_version,
        digest: sha256_hex(&bytes),
        byte_length: bytes.len() as u64,
    };
    Ok((bytes, packed))
}

/// Unpack a package back to its network, verifying every attestation.
pub fn unpack_network(bytes: &[u8]) -> Result<WaterNetworkSnapshot, WaterPackageError> {
    let package = unpack(bytes)?;
    ensure_application_package(&package)?;
    let by_leaf = package
        .assets
        .iter()
        .map(|asset| (asset.invariant_leaf.as_str(), asset.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    if by_leaf.len() != 2 {
        return Err(WaterPackageError::Invalid(
            "package carries undeclared Water network assets".into(),
        ));
    }
    let descriptor: Descriptor = serde_json::from_slice(
        by_leaf
            .get(DESCRIPTOR_LEAF)
            .ok_or_else(|| WaterPackageError::Invalid("profile descriptor is missing".into()))?,
    )
    .map_err(json)?;
    if descriptor.profile != PROFILE
        || descriptor.profile_version != PROFILE_VERSION
        || descriptor.object_type != "water_network"
        || descriptor.source_revision == 0
        || descriptor.network.leaf != NETWORK_LEAF
        || descriptor.network.encoding != "json+zstd"
    {
        return Err(WaterPackageError::Invalid(
            "unsupported or malformed Water network profile descriptor".into(),
        ));
    }
    if package.manifest.model.model_revision != descriptor.source_revision
        || package.manifest.model.model_id.as_str() != manifest_model_id(&descriptor.object_id)
    {
        return Err(WaterPackageError::Invalid(
            "profile identity disagrees with the native package manifest".into(),
        ));
    }
    let frame = by_leaf
        .get(NETWORK_LEAF)
        .copied()
        .ok_or_else(|| WaterPackageError::Invalid("the network asset is missing".into()))?;
    if frame.len() as u64 != descriptor.network.byte_len
        || sha256_hex(frame) != descriptor.network.sha256
    {
        return Err(WaterPackageError::Invalid(
            "the network asset disagrees with its profile attestation".into(),
        ));
    }
    let frame_error = |e: String| WaterPackageError::Invalid(format!("network frame: {e}"));
    let decoder =
        ruzstd::decoding::StreamingDecoder::new(frame).map_err(|e| frame_error(e.to_string()))?;
    let mut network_json = Vec::new();
    decoder
        .take(MAX_NETWORK_JSON_BYTES + 1)
        .read_to_end(&mut network_json)
        .map_err(|e| frame_error(e.to_string()))?;
    if network_json.len() as u64 > MAX_NETWORK_JSON_BYTES {
        return Err(frame_error("decodes past its bound".into()));
    }
    Ok(WaterNetworkSnapshot {
        source_project_id: descriptor.source_project_id,
        model_id: descriptor.object_id,
        source_revision: descriptor.source_revision,
        network: serde_json::from_slice(&network_json).map_err(json)?,
    })
}

fn ensure_application_package(package: &GridPackage) -> Result<(), WaterPackageError> {
    if TableKind::ALL
        .iter()
        .any(|kind| package.snapshot.table_count(*kind) != 0)
    {
        return Err(WaterPackageError::Invalid(
            "a Water network package must not carry canonical grid model rows".into(),
        ));
    }
    if package.manifest.model.coordinate_system.as_str() != APPLICATION_CRS {
        return Err(WaterPackageError::Invalid(
            "package is not a Water network application snapshot".into(),
        ));
    }
    Ok(())
}

fn validate_identity(snapshot: &WaterNetworkSnapshot) -> Result<(), WaterPackageError> {
    let clean = |text: &str| {
        !text.trim().is_empty() && text == text.trim() && !text.chars().any(char::is_control)
    };
    if !clean(&snapshot.model_id) {
        return Err(WaterPackageError::Invalid(
            "model_id must be non-empty, trimmed, and control-free".into(),
        ));
    }
    if snapshot
        .source_project_id
        .as_deref()
        .is_some_and(|p| !clean(p))
    {
        return Err(WaterPackageError::Invalid(
            "source_project_id, when present, must be trimmed and control-free".into(),
        ));
    }
    if snapshot.source_revision == 0 {
        return Err(WaterPackageError::Invalid(
            "source_revision must be positive".into(),
        ));
    }
    Ok(())
}

/// The native manifest model id of a catalog model id.
pub fn manifest_model_id(model_id: &str) -> String {
    format!("water-network-{}", sha256_hex(model_id.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
