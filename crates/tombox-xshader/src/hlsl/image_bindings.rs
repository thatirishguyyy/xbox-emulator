//! Resolve MIMG descriptor-table entries using TomBox's PS09 register-space matching.
use super::Emitter;
use crate::psocache::UserDataKind;
use crate::Stage;

impl Emitter<'_> {
    pub(super) fn direct_raw_binding(&self, sgpr: u8) -> Option<u32> {
        if !self.is_ms() || self.ex.root_sig.is_some() || self.image_layouts.is_empty() {
            return None;
        }
        let mut agreed = None;
        for layout in self.image_layouts {
            let mut entries = layout.entries.iter().filter(|entry| {
                entry.user_sgpr().checked_add(8) == Some(sgpr)
                    && entry.byte_offset == 0
                    && entry.kind == UserDataKind::Other(0x100)
                    && entry.descriptor_dwords() == 2
            });
            let entry = entries.next()?;
            if entries.next().is_some()
                || entry.register > 0xffff
                || entry.register_space() >= 0x8000
                || !self.ex.resources.iter().any(|resource| {
                    resource.res_type == 4
                        && resource.space == entry.register_space()
                        && (resource.lower..=resource.upper).contains(&entry.register)
                })
            {
                return None;
            }
            let slot = (entry.register_space() << 16) | entry.register;
            if agreed.replace(slot).is_some_and(|previous| previous != slot) {
                return None;
            }
        }
        agreed
    }

    pub(super) fn image_binding(&self, table: u8, offset: u32, types: &[u32]) -> Result<Option<(u32, u32)>, String> {
        let mut resolved = None;
        for layout in self.image_layouts {
            let mut candidate = None;
            for entry in &layout.entries {
                let resource_type = match entry.kind {
                    UserDataKind::Sampler if entry.static_sampler_words().is_none() => 1,
                    UserDataKind::ShaderResource => 3,
                    UserDataKind::UnorderedAccess => 6,
                    _ => continue,
                };
                let base = if self.ex.stage == Stage::Mesh { 8 } else { 0 };
                if entry.user_sgpr().checked_add(base) != Some(table) || entry.byte_offset != offset || !types.contains(&resource_type) {
                    continue;
                }
                let matching = self.ex.resources.iter().filter(|r| {
                    r.res_type == resource_type && r.lower <= entry.register && entry.register <= r.upper
                });
                let mut hinted = matching.clone().filter(|r| r.space == entry.register_space());
                let resource = if let Some(first) = hinted.next() {
                    if hinted.next().is_some() { return Err(format!("PSO: ambiguous resource for s{table}+{offset:#x}")); }
                    first
                } else {
                    let mut all = matching;
                    let Some(first) = all.next() else { continue };
                    if all.next().is_some() { return Err(format!("PSO: ambiguous register space for s{table}+{offset:#x}")); }
                    first
                };
                if resource.space >= 0x8000 || entry.register > 0xffff {
                    return Err("PSO: image binding exceeds packed register range".into());
                }
                let binding = (resource.space << 16 | entry.register, resource_type);
                if candidate.replace(binding).is_some_and(|prev| prev != binding) {
                    return Err(format!("PSO: conflicting image descriptors for s{table}+{offset:#x}"));
                }
            }
            let binding = candidate.ok_or_else(|| format!("PSO: no image descriptor for s{table}+{offset:#x}"))?;
            if resolved.replace(binding).is_some_and(|prev| prev != binding) {
                return Err(format!("PSO: conflicting image layouts for s{table}+{offset:#x}"));
            }
        }
        Ok(resolved)
    }
}
