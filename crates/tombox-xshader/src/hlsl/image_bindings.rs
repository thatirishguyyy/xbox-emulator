//! Resolve MIMG descriptor-table entries using TomBox's PS09 register-space matching.
use super::{Emitter, Sym, BINDLESS};
use crate::psocache::UserDataKind;
use crate::Stage;

impl Emitter<'_> {
    pub(super) fn raw_binding(&self, symbol: Option<Sym>) -> Option<u32> {
        let (sgpr, offset, kind, dwords) = match symbol {
            Some(Sym::Table(root)) => (root, 0, UserDataKind::Other(0x100), 2),
            Some(Sym::Desc { table, off, k: 0 }) if off & BINDLESS == 0 => {
                (table, off, UserDataKind::ShaderResource, 4)
            }
            _ => return None,
        };
        self.mesh_buffer_binding(sgpr, offset, kind, dwords, 4).map(|(slot, _)| slot)
    }

    pub(super) fn constant_binding(&self, table: u8, offset: u32) -> Option<u32> {
        self.mesh_buffer_binding(table, offset, UserDataKind::ConstantBuffer, 4, 2).map(|(slot, _)| slot)
    }

    pub(super) fn structured_binding(&self, table: u8, offset: u32) -> Option<(u32, u32)> {
        self.mesh_buffer_binding(table, offset, UserDataKind::ShaderResource, 4, 5)
    }

    fn mesh_buffer_binding(
        &self, sgpr: u8, offset: u32, kind: UserDataKind, dwords: u8, resource_type: u32,
    ) -> Option<(u32, u32)> {
        if !self.is_ms() || self.ex.root_sig.is_some() || offset & BINDLESS != 0 {
            return None;
        }
        let mut agreed = None;
        for layout in self.image_layouts {
            let mut entries = layout.entries.iter().filter(|entry| {
                entry.user_sgpr().checked_add(8) == Some(sgpr)
                    && entry.byte_offset == offset
                    && entry.kind == kind
                    && entry.descriptor_dwords() == dwords
            });
            let entry = entries.next()?;
            if entries.next().is_some()
                || entry.register > 0xffff
                || entry.register_space() >= 0x8000
                || !self.ex.resources.iter().any(|resource| {
                    resource.res_type == resource_type
                        && resource.space == entry.register_space()
                        && (resource.lower..=resource.upper).contains(&entry.register)
                })
            {
                return None;
            }
            let slot = (entry.register_space() << 16) | entry.register;
            // For structured SRVs, the first PS09 field appears to carry the byte stride.
            let stride = if resource_type == 5 {
                let bytes = entry.raw_fields[0];
                if !(4..=2048).contains(&bytes) || bytes % 4 != 0 {
                    return None;
                }
                bytes / 4
            } else {
                0
            };
            let binding = (slot, stride);
            if agreed.replace(binding).is_some_and(|previous| previous != binding) {
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
