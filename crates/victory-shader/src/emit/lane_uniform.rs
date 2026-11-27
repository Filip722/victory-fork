use super::*;

/// VGPR values that are the same in every lane holding them (`all` is [`all_lane_blocks`]):
/// written by an instruction every arriving lane runs (no guard), lane by lane, from scalars,
/// constants and such values; or loaded from an address made of them. The compiler keeps
/// wave-uniform data in VGPRs and fetches it with `v_readfirstlane`.
/// shortcut: a value joined by a phi counts only where every predecessor is a block all lanes
/// run (no lane branch can have split the arriving lanes); loop-carried values never do.
pub(super) fn lane_uniform(p: &Program, st: &Structured, all: &[bool]) -> HashSet<Value> {
    let is_mask = |v: Value| st.masks.contains_key(&v) || st.highs.contains_key(&v);
    let mut same: HashSet<Value> = HashSet::new();
    for b in crate::ir::reverse_postorder(&p.blocks) {
        let block = &p.blocks[b];
        let whole = block.preds.iter().all(|&q| all.get(q).copied().unwrap_or(false));
        for phi in &block.phis {
            if matches!(phi.reg, Reg::V(_)) && whole && phi.args.iter().all(|a| same.contains(a)) {
                same.insert(phi.value);
            }
        }
        for (i, inst) in block.insts.iter().enumerate() {
            let name = inst.name();
            if st.guards[b][i].is_some() {
                continue; // lanes outside EXEC keep another value
            }
            let alike = match &inst.fields {
                Fields::Valu(v)
                    if v.dpp.is_none()
                        && v.sdwa.is_none()
                        && !name.starts_with("v_mbcnt")
                        && !name.starts_with("v_cmpx")
                        && !name.contains("lane") =>
                {
                    v.src.iter().all(|op| match *op {
                        Operand::Vector { reg, dwords } => {
                            // every dword of the operand the instruction reads (not the old value
                            // of a higher-numbered destination register that IR keeps for guards)
                            // A relative move reads the register `M0` past this one, and a
                            // swap or a relative store keeps or moves old values: for those
                            // every register read from this one on counts, as before.
                            let whole = name.starts_with("v_movrel") || name.starts_with("v_swap");
                            let end = reg.saturating_add(dwords);
                            let read: Vec<Value> = inst
                                .reads
                                .iter()
                                .filter(|r| matches!(r.0, Reg::V(x) if x >= reg && (whole || x < end)))
                                .map(|r| r.1)
                                .collect();
                            !read.is_empty() && read.iter().all(|x| same.contains(x))
                        }
                        // a scalar is one value; a lane mask is not, unless a compare of such values made it
                        Operand::Scalar { code, dwords } if code != NULL => {
                            (code..code.saturating_add(dwords)).all(|c| {
                                inst.read(Reg::S(c)).is_none_or(|x| !is_mask(x) || same.contains(&x))
                            })
                        }
                        _ => true,
                    })
                }
                Fields::Ds {
                    addr: Some(Operand::Vector { reg, .. }),
                    gds: false,
                    ..
                } if name.starts_with("ds_read") && !name.contains("_d16") => {
                    // An unguarded LDS read from a wave-uniform address gives the same value
                    // in every arriving lane. `_d16` forms preserve the other half of the
                    // destination register and are left out.
                    inst.read(Reg::V(*reg)).is_some_and(|x| same.contains(&x))
                }
                _ => false,
            };
            if alike {
                same.extend(inst.writes.iter().filter(|w| w.0 != Reg::Scc).map(|w| w.1));
            }
        }
    }
    same
}
