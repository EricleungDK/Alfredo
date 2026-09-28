//! Read-only, non-atomic observations of selected-model metadata, never a runtime pin.
//! Field contracts checked against official Ollama docs on 2026-09-26:
//! https://docs.ollama.com/api/tags and https://docs.ollama.com/api/ps
//! https://github.com/ollama/ollama/blob/main/docs/api.md#version
use crate::provider::Ollama;
use serde::{
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Serialize,
};
use serde_json::{Map, Value};
use std::{fmt, time::Duration};
const MAX_BYTES: usize = 1024 * 1024;
const MAX_CONTEXT: u64 = 1 << 24;
const MAX_MODEL_BYTES: u64 = 1 << 60;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    pub digest: Option<String>,
    pub quantization: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunningModel {
    pub digest: Option<String>,
    pub context_length: Option<u64>,
    pub size: Option<u64>,
    pub size_vram: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub endpoint_origin: String,
    pub selected_model: String,
    pub server_version: Option<String>,
    pub catalog: Option<CatalogModel>,
    pub running: Option<RunningModel>,
}
impl Observation {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.endpoint_origin)
            .map_err(|_| "Invalid runtime endpoint origin")?;
        if self.endpoint_origin.len() > 2048
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.origin().ascii_serialization() != self.endpoint_origin
            || !text(&self.selected_model, 200)
            || self.server_version.as_ref().is_some_and(|s| !text(s, 128))
        {
            return Err("Invalid bounded runtime identity".into());
        }
        if self.catalog.as_ref().is_some_and(|catalog| {
            catalog.digest.as_ref().is_some_and(|s| !digest(s))
                || catalog.quantization.as_ref().is_some_and(|s| !text(s, 64))
        }) {
            return Err("Invalid catalog digest or quantization".into());
        }
        if let Some(running) = &self.running {
            if running.digest.as_ref().is_some_and(|s| !digest(s))
                || running
                    .context_length
                    .is_some_and(|n| n == 0 || n > MAX_CONTEXT)
                || running.size.is_some_and(|n| n > MAX_MODEL_BYTES)
                || running.size_vram.is_some_and(|n| n > MAX_MODEL_BYTES)
                || matches!((running.size, running.size_vram), (Some(size),Some(vram)) if vram > size)
            {
                return Err("Invalid running model digest, context or byte counts".into());
            }
            if let (Some(catalog), Some(observed)) = (
                self.catalog.as_ref().and_then(|c| c.digest.as_ref()),
                running.digest.as_ref(),
            ) {
                if catalog != observed {
                    return Err(
                        "Running model digest does not match the selected catalog model".into(),
                    );
                }
            }
        }
        Ok(())
    }
    /// Missing proof is explicit. An absent pre-run resident is not by itself a
    /// reason to skip a cohort; qualification policy decides which phase needs it.
    pub fn missing_reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if let Err(error) = self.validate() {
            reasons.push(error);
        }
        if self.server_version.is_none() {
            reasons.push("Server version not observed".into());
        }
        match &self.catalog {
            None => reasons.push("Selected model not observed in catalog".into()),
            Some(catalog) => {
                if catalog.digest.is_none() {
                    reasons.push("Catalog model digest not observed".into());
                }
                if catalog.quantization.is_none() {
                    reasons.push("Catalog quantization not observed".into());
                }
            }
        }
        match &self.running {
            None => reasons.push("Selected model not observed running".into()),
            Some(running) => {
                if running.digest.is_none() {
                    reasons.push("Running model digest not observed".into());
                }
                if running.context_length.is_none() {
                    reasons.push("Running context length not observed".into());
                }
                if running.size.is_none() {
                    reasons.push("Running total bytes not observed".into());
                }
                if running.size_vram.is_none() {
                    reasons.push("Running GPU bytes not observed".into());
                }
            }
        }
        reasons
    }
    /// Completeness and exact request binding for a post-request observation.
    /// An empty list confirms these observations, not an atomic upstream runtime pin.
    pub fn request_reasons(
        &self,
        endpoint_origin: &str,
        selected_model: &str,
        requested_context: Option<u32>,
    ) -> Vec<String> {
        let mut reasons = self.missing_reasons();
        if self.endpoint_origin != endpoint_origin {
            reasons.push("Runtime endpoint differs from the generation request".into());
        }
        if self.selected_model != selected_model {
            reasons.push("Runtime selected model differs from the generation request".into());
        }
        if let Some(context) = requested_context {
            if self
                .running
                .as_ref()
                .and_then(|running| running.context_length)
                != Some(u64::from(context))
            {
                reasons
                    .push("Observed running context does not confirm the requested profile".into());
            }
        }
        reasons.sort();
        reasons.dedup();
        reasons
    }
    /// Loading/unloading and requested context changes are expected observations,
    /// not identity drift. Version, selected identity and catalog facts must stay stable.
    pub fn drift_reasons(&self, after: &Self) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.endpoint_origin != after.endpoint_origin {
            reasons.push("Endpoint origin changed".into());
        }
        if self.selected_model != after.selected_model {
            reasons.push("Selected model changed".into());
        }
        if self.server_version != after.server_version {
            reasons.push("Server version changed or became unknown".into());
        }
        if self.catalog.as_ref().and_then(|c| c.digest.as_ref())
            != after.catalog.as_ref().and_then(|c| c.digest.as_ref())
        {
            reasons.push("Catalog model digest changed or became unknown".into());
        }
        if self.catalog.as_ref().and_then(|c| c.quantization.as_ref())
            != after.catalog.as_ref().and_then(|c| c.quantization.as_ref())
        {
            reasons.push("Catalog quantization changed or became unknown".into());
        }
        if let (Some(before), Some(after)) = (
            self.running.as_ref().and_then(|r| r.digest.as_ref()),
            after.running.as_ref().and_then(|r| r.digest.as_ref()),
        ) {
            if before != after {
                reasons.push("Running model digest changed".into());
            }
        }
        reasons
    }
}
fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Three bounded GETs use the configured provider transport. They acquire no
/// inference permit and neither load models nor create mission/coordinator state.
pub async fn inspect(provider: &Ollama, model: &str) -> Result<Observation> {
    if !text(model, 200) {
        return Err("Selected runtime model must contain 1–200 bytes without controls".into());
    }
    let (version, catalog, running) = tokio::try_join!(
        read(provider, "/api/version"),
        read(provider, "/api/tags"),
        read(provider, "/api/ps")
    )?;
    let version = object(&version, "version response")?;
    let catalog = selected(&catalog, model)?;
    let running = selected(&running, model)?;
    let observation = Observation {
        endpoint_origin: provider.endpoint_origin(),
        selected_model: model.into(),
        server_version: string(version, "version", 128)?.map(str::to_owned),
        catalog: catalog
            .map(|entry| -> Result<CatalogModel> {
                let details = match entry.get("details") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(object(value, "model details")?),
                };
                Ok(CatalogModel {
                    digest: string(entry, "digest", 64)?.map(str::to_owned),
                    quantization: details
                        .map(|d| string(d, "quantization_level", 64))
                        .transpose()?
                        .flatten()
                        .map(str::to_owned),
                })
            })
            .transpose()?,
        running: running
            .map(|entry| -> Result<RunningModel> {
                Ok(RunningModel {
                    digest: string(entry, "digest", 64)?.map(str::to_owned),
                    context_length: number(entry, "context_length")?,
                    size: number(entry, "size")?,
                    size_vram: number(entry, "size_vram")?,
                })
            })
            .transpose()?,
    };
    observation.validate()?;
    Ok(observation)
}
async fn read(provider: &Ollama, path: &str) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut url = reqwest::Url::parse(&provider.endpoint_origin())
            .map_err(|_| "Invalid provider origin")?;
        url.set_path(path);
        let mut response = provider
            .inspection_client()
            .get(url)
            .send()
            .await
            .map_err(|_| format!("Runtime inspection {path} transport failed"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Runtime inspection {path} returned HTTP {}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(format!("Runtime inspection {path} exceeds 1 MiB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| format!("Runtime inspection {path} connection lost"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err(format!("Runtime inspection {path} exceeds 1 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
        let mut nodes = 0;
        let value = Bounded {
            depth: 0,
            nodes: &mut nodes,
        }
        .deserialize(&mut deserializer)
        .map_err(|error| format!("Invalid runtime inspection {path}: {error}"))?;
        deserializer
            .end()
            .map_err(|_| format!("Runtime inspection {path} contains trailing data"))?;
        Ok(value)
    })
    .await
    .map_err(|_| format!("Runtime inspection {path} exceeded its ten-second deadline"))?
}
fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| format!("Runtime {label} must be an object"))
}
fn string<'a>(object: &'a Map<String, Value>, key: &str, limit: usize) -> Result<Option<&'a str>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if text(value, limit) => Ok(Some(value)),
        _ => Err(format!("Invalid runtime metadata field {key}")),
    }
}
fn number(object: &Map<String, Value>, key: &str) -> Result<Option<u64>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("Runtime metadata {key} must be an unsigned integer")),
    }
}
fn selected<'a>(value: &'a Value, selected: &str) -> Result<Option<&'a Map<String, Value>>> {
    let root = object(value, "model list")?;
    let models = match root.get("models") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(models)) => models,
        _ => return Err("Runtime model list must be an array".into()),
    };
    let mut result = None;
    for value in models {
        let entry = object(value, "model entry")?;
        let name = string(entry, "name", 200)?;
        let model = string(entry, "model", 200)?;
        if name.is_none() && model.is_none() {
            return Err("Runtime model entry has no model identity".into());
        }
        if name == Some(selected) || model == Some(selected) {
            if matches!((name, model), (Some(name),Some(model)) if name != model) {
                return Err("Selected runtime model name and model identities disagree".into());
            }
            if result.replace(entry).is_some() {
                return Err("Selected runtime model identity is duplicated".into());
            }
        }
    }
    Ok(result)
}
/// Reject duplicate keys and arbitrary JSON growth before retaining only the allowlist.
struct Bounded<'a> {
    depth: usize,
    nodes: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for Bounded<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        if self.depth > 8 || *self.nodes >= 8192 {
            return Err(serde::de::Error::custom(
                "metadata depth or node bound exceeded",
            ));
        }
        *self.nodes += 1;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Bounded<'_> {
    type Value = Value;
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("bounded metadata JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite metadata number"))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Value, E> {
        self.visit_string(value.into())
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> std::result::Result<Value, E> {
        if value.len() > 4096 {
            return Err(E::custom("metadata string exceeds 4096 bytes"));
        }
        Ok(Value::String(value))
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(Bounded {
            depth: self.depth + 1,
            nodes: self.nodes,
        })? {
            if values.len() == 256 {
                return Err(serde::de::Error::custom(
                    "metadata collection exceeds 256 entries",
                ));
            }
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> std::result::Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.len() == 64 || key.len() > 128 || values.contains_key(&key) {
                return Err(serde::de::Error::custom(
                    "metadata object bound or duplicate key",
                ));
            }
            let value = object.next_value_seed(Bounded {
                depth: self.depth + 1,
                nodes: self.nodes,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
