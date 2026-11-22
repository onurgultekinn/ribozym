//! A12 — Archive & courier layer (§7.6, v2.47 B2). "Every morning a new
//! worker, holding yesterday's minutes": the run's context is the ARCHIVE,
//! not a conversation. Deterministic by design (§5.8 m4 — collection,
//! hashing, stamping are better done by code than by any model).
//!
//! Layout: `runs/<spec_id>/archive/NNN-name` + `manifest.json` (section,
//! sha256, size, ts per entry). Integrity rule B2: EVERY read verifies the
//! manifest hash — a mismatch is never silently skipped (`ArchiveCorrupt`):
//! if the source document still exists the archive is rebuildable
//! (`rebuild`), otherwise the run parks for the human.
//!
//! Deviation note (honest, §19 plan-doc): the doc names zstd sections; the
//! offline dependency set has no compression crate, so sections are stored
//! as canonical bytes. The manifest hash protocol is identical, so switching
//! to zstd later is a storage-format change, NOT an integrity change.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hash;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Section {
    /// e.g. "001-raw_input.md", "002-spec.json"
    pub name: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub ts: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema_version: String,
    pub spec_id: String,
    pub sections: Vec<Section>,
}

pub struct Archive {
    dir: PathBuf,
    spec_id: String,
}

impl Archive {
    pub fn open(root: &Path, spec_id: &str) -> Self {
        Self {
            dir: root.join("runs").join(spec_id).join("archive"),
            spec_id: spec_id.to_string(),
        }
    }

    pub fn path_of(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    pub fn manifest(&self) -> Result<Manifest> {
        match std::fs::read(self.manifest_path()) {
            Ok(bytes) => {
                let m: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
                    Error::ConfigInvalid(format!("archive manifest unreadable: {e}"))
                })?;
                Ok(m)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest {
                schema_version: "1.0".into(),
                spec_id: self.spec_id.clone(),
                sections: vec![],
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// Write one section under an explicit ordinal name, appending the
    /// verified record to the manifest. Re-writing the SAME name with
    /// DIFFERENT bytes is refused: a sealed run document may be resealed
    /// (identical content) but never silently rewritten under the same
    /// section — that would unseat the audit trail.
    pub fn write_section(&self, name: &str, bytes: &[u8], ts: &str) -> Result<Section> {
        std::fs::create_dir_all(&self.dir)?;
        let sha = format!("sha256:{}", hash::sha256_hex(bytes));
        let mut m = self.manifest()?;
        if let Some(prev) = m.sections.iter().find(|s| s.name == name) {
            if prev.sha256 != sha {
                return Err(Error::ConfigInvalid(format!(
                    "archive section {name:?} already sealed with different bytes — \
                     immutable record violated"
                )));
            }
            return Ok(prev.clone());
        }
        let path = self.path_of(name);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&path)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        // Read-back integrity: what we promised is what landed.
        let landed = std::fs::read(&path)?;
        let landed_sha = format!("sha256:{}", hash::sha256_hex(&landed));
        if landed_sha != sha {
            return Err(Error::ConfigInvalid(format!(
                "ARCHIVE_CORRUPT: {name} read-back hash mismatch immediately after write"
            )));
        }
        let rec = Section {
            name: name.to_string(),
            sha256: sha,
            size_bytes: bytes.len() as u64,
            ts: ts.to_string(),
        };
        m.sections.push(rec.clone());
        if m.schema_version.is_empty() {
            m.schema_version = "1.0".into();
        }
        m.spec_id = self.spec_id.clone();
        let doc = serde_json::to_value(&m)?;
        std::fs::write(self.manifest_path(), hash::canonical_json(&doc))?;
        Ok(rec)
    }

    /// Append the NEXT ordinal section for a logical label. Identical bytes
    /// re-seal idempotently; changed bytes get the next number instead of
    /// breaking immutability — the sealed report may be superseded by a
    /// re-review, and §7.6 forbids rewriting history, not recording more
    /// of it (`005-report.json` → `007-report.json`).
    pub fn append(&self, label: &str, bytes: &[u8], ts: &str) -> Result<Section> {
        let m = self.manifest()?;
        let latest = m.sections.iter().rev().find(|s| {
            s.name
                .split_once('-')
                .map(|(_, l)| l == label)
                .unwrap_or(false)
        });
        let sha = format!("sha256:{}", hash::sha256_hex(bytes));
        if let Some(rec) = latest {
            if rec.sha256 == sha {
                return Ok(rec.clone());
            }
        }
        let next = m
            .sections
            .iter()
            .filter_map(|s| s.name.split('-').next()?.parse::<u32>().ok())
            .max()
            .map_or(1, |x| x + 1);
        self.write_section(&format!("{next:03}-{label}"), bytes, ts)
    }

    /// Does the current bytes match ANY sealed entry for this label
    /// (append-semantics verification for superseded documents)?
    pub fn matches_any(&self, label: &str, bytes: &[u8]) -> Result<bool> {
        let m = self.manifest()?;
        let sha = format!("sha256:{}", hash::sha256_hex(bytes));
        let any = m.sections.iter().any(|s| {
            s.name
                .split_once('-')
                .map(|(_, l)| l == label)
                .unwrap_or(false)
        });
        if !any {
            return Ok(true); // not sealed yet: nothing to contradict
        }
        Ok(m.sections
            .iter()
            .any(|s| s.name.ends_with(label) && s.sha256 == sha))
    }

    /// §7.6 B2: a verified read. Unknown name, missing file or hash drift is
    /// `ArchiveCorrupt` — silent continue is forbidden.
    pub fn read_section(&self, name: &str) -> Result<Vec<u8>> {
        let m = self.manifest()?;
        let rec = m
            .sections
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| Error::ArchiveCorrupt(format!("no manifest entry for {name}")))?;
        let bytes = std::fs::read(self.path_of(name))
            .map_err(|e| Error::ArchiveCorrupt(format!("{name} unreadable: {e}")))?;
        let sha = format!("sha256:{}", hash::sha256_hex(&bytes));
        if sha != rec.sha256 {
            return Err(Error::ArchiveCorrupt(format!(
                "{name}: manifest says {} but content hashes to {sha}",
                rec.sha256
            )));
        }
        Ok(bytes)
    }

    /// Full sweep (`ribozym` calls it at resume start): every recorded
    /// section verified; returns the corrupt names (empty = healthy).
    pub fn verify_all(&self) -> Result<Vec<String>> {
        let m = self.manifest()?;
        let mut bad = Vec::new();
        for rec in &m.sections {
            match std::fs::read(self.path_of(&rec.name)) {
                Ok(bytes) => {
                    let sha = format!("sha256:{}", hash::sha256_hex(&bytes));
                    if sha != rec.sha256 {
                        bad.push(rec.name.clone());
                    }
                }
                Err(_) => bad.push(rec.name.clone()),
            }
        }
        Ok(bad)
    }

    /// Re-seal a corrupt/missing section from a live source document (the
    /// B2 "üretilebiliyorsa koddan yeniden yazılır" branch). Returns false
    /// when the source is gone too — the run must then park for the human.
    pub fn repair_section(&self, name: &str, source: &Path, ts: &str) -> Result<bool> {
        if !source.exists() {
            return Ok(false);
        }
        let bytes = std::fs::read(source)?;
        // Replace the record: drop the stale entry, re-write, re-seal.
        let mut m = self.manifest()?;
        m.sections.retain(|s| s.name != name);
        let doc = serde_json::to_value(&m)?;
        std::fs::write(self.manifest_path(), hash::canonical_json(&doc))?;
        self.write_section(name, &bytes, ts)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_verify_round_trip_and_corruption_detection() {
        let dir = tempfile::tempdir().unwrap();
        let a = Archive::open(dir.path(), "spec-20260912-001");
        let content = b"{\"title\":\"tiny\"}".to_vec();
        a.write_section("002-spec.json", &content, "2026-09-12T10:00:00Z")
            .unwrap();
        assert_eq!(a.read_section("002-spec.json").unwrap(), content);
        assert!(a.verify_all().unwrap().is_empty());
        // Corrupt the section file under the manifest's nose...
        std::fs::write(a.path_of("002-spec.json"), b"{\"title\":\"TAMPERED\"}").unwrap();
        let err = a.read_section("002-spec.json").unwrap_err();
        assert!(err.to_string().contains("ARCHIVE_CORRUPT"), "{err}");
        let bad = a.verify_all().unwrap();
        assert_eq!(bad, vec!["002-spec.json".to_string()]);
    }

    #[test]
    fn same_bytes_reseal_is_idempotent_different_bytes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = Archive::open(dir.path(), "spec-20260912-001");
        a.write_section("001-raw_input.md", b"form v1", "t1")
            .unwrap();
        let again = a
            .write_section("001-raw_input.md", b"form v1", "t1")
            .unwrap();
        assert_eq!(
            again.ts, "t1",
            "idempotent re-seal keeps the original stamp"
        );
        let err = a
            .write_section("001-raw_input.md", b"form v2", "t2")
            .unwrap_err();
        assert!(err.to_string().contains("immutable"), "{err}");
    }

    #[test]
    fn repair_reseals_from_a_live_source_and_says_no_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let a = Archive::open(dir.path(), "spec-20260912-001");
        let content = b"canonical report".to_vec();
        a.write_section("004-report.json", &content, "t1").unwrap();
        std::fs::write(a.path_of("004-report.json"), b"mutilated").unwrap();
        assert!(!a.verify_all().unwrap().is_empty());
        let source = dir.path().join("source_report.bin");
        std::fs::write(&source, &content).unwrap();
        assert!(a.repair_section("004-report.json", &source, "t2").unwrap());
        assert!(
            a.verify_all().unwrap().is_empty(),
            "repaired from the source"
        );
        assert!(
            !a.repair_section("009-plan.json", &dir.path().join("gone.json"), "t")
                .unwrap(),
            "no source to rebuild from → the caller must park for the human"
        );
    }

    #[test]
    fn append_supersedes_with_the_next_ordinal_never_rewrites_history() {
        let dir = tempfile::tempdir().unwrap();
        let a = Archive::open(dir.path(), "spec-20260912-001");
        a.append("report.json", b"v1", "t1").unwrap();
        a.append("report.json", b"v1", "t1").unwrap(); // idempotent
        let m = a.manifest().unwrap();
        assert_eq!(m.sections.len(), 1);
        a.append("report.json", b"v2 sealed later", "t2").unwrap();
        let m = a.manifest().unwrap();
        assert_eq!(m.sections.len(), 2, "changed bytes append, never rewrite");
        assert_eq!(m.sections[0].name, "001-report.json");
        assert_eq!(m.sections[1].name, "002-report.json");
        assert!(a.matches_any("report.json", b"v1").unwrap());
        assert!(a.matches_any("report.json", b"v2 sealed later").unwrap());
        assert!(!a.matches_any("report.json", b"tampered").unwrap());
        assert!(a.verify_all().unwrap().is_empty());
    }

    #[test]
    fn missing_manifest_entry_is_corrupt_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let a = Archive::open(dir.path(), "spec-20260912-001");
        let err = a.read_section("000-nope.md").unwrap_err();
        assert!(err.to_string().contains("ARCHIVE_CORRUPT"), "{err}");
    }
}
