use super::{Buf, Bytes, CargoDep, CargoIndexEntry};

/// Parse the Cargo publish wire format:
/// `[4B LE u32 meta_len][JSON][4B LE u32 crate_len][.crate bytes]`
pub(super) fn parse_publish_body(mut body: Bytes) -> Result<(serde_json::Value, Bytes), String> {
    if body.remaining() < 4 {
        return Err("publish body too short (missing metadata length)".into());
    }
    let meta_len = body.get_u32_le() as usize;
    if body.remaining() < meta_len {
        return Err(format!(
            "metadata length {meta_len} exceeds remaining body ({} bytes)",
            body.remaining()
        ));
    }
    let meta_bytes = body.copy_to_bytes(meta_len);
    let meta_json: serde_json::Value = serde_json::from_slice(&meta_bytes)
        .map_err(|e| format!("invalid publish metadata JSON: {e}"))?;

    if body.remaining() < 4 {
        return Err("publish body too short (missing crate length)".into());
    }
    let crate_len = body.get_u32_le() as usize;
    if body.remaining() < crate_len {
        return Err(format!(
            "crate length {crate_len} exceeds remaining body ({} bytes)",
            body.remaining()
        ));
    }
    let crate_bytes = body.copy_to_bytes(crate_len);

    Ok((meta_json, crate_bytes))
}

/// Convert a Cargo publish metadata JSON object into a sparse index `CargoIndexEntry`.
/// The publish format uses `version_req`; the index format uses `req`.
pub(super) fn metadata_to_index_entry(
    meta: &serde_json::Value,
    checksum: &str,
) -> Result<CargoIndexEntry, String> {
    let name = meta["name"].as_str().ok_or("missing 'name'")?.to_owned();
    let vers = meta["vers"].as_str().ok_or("missing 'vers'")?.to_owned();

    let deps = meta
        .get("deps")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|dep| {
                    Ok(CargoDep {
                        name: dep["name"].as_str().ok_or("dep missing 'name'")?.to_owned(),
                        req: dep
                            .get("version_req")
                            .and_then(|v| v.as_str())
                            .or_else(|| dep.get("req").and_then(|v| v.as_str()))
                            .ok_or("dep missing 'version_req'")?
                            .to_owned(),
                        features: dep
                            .get("features")
                            .and_then(|f| f.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        optional: dep
                            .get("optional")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        default_features: dep
                            .get("default_features")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                        target: dep
                            .get("target")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        kind: dep
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("normal")
                            .to_owned(),
                        registry: dep
                            .get("registry")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        explicit_name_in_toml: dep
                            .get("explicit_name_in_toml")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    })
                })
                .collect::<Result<Vec<_>, &str>>()
        })
        .transpose()
        .map_err(|e: &str| e.to_owned())?
        .unwrap_or_default();

    let features = meta
        .get("features")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let links = meta
        .get("links")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let rust_version = meta
        .get("rust_version")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    Ok(CargoIndexEntry {
        name,
        vers,
        deps,
        cksum: checksum.to_owned(),
        features,
        features2: None,
        yanked: false,
        links,
        rust_version,
        v: None,
    })
}
