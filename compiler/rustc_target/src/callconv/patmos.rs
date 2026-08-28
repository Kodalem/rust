use rustc_abi::{HasDataLayout, Size};
use rustc_abi::TyAbiInterface;

// PassMode, ArgAttributes, CastTarget, Reg, and Uniform from localized scope
use crate::callconv::{ArgAbi, FnAbi, PassMode, ArgAttributes, CastTarget, Reg, Uniform};

/// Patmos ABI implementation.
/// Based on the Patmos C calling convention as defined in the Patmos compiler.
///
/// Register allocation:
/// - Return: r1 (and r2 for values > 4 bytes)
/// - Arguments: r3, r4, r5, r6, r7, r8 (6 registers)
/// - Stack: 4-byte aligned, 4-byte slots
///
/// Type handling:
/// - All integers extended to 32-bit (Patmos has no sub-word operations)
/// - Aggregates > 16 bytes OR > 6 words (24 bytes) passed indirectly
/// - Scalars use registers then spill to stack
fn classify_arg<'a, Ty, C>(cx: &C, arg: &mut ArgAbi<'a, Ty>, state: &mut usize)
where
    Ty: TyAbiInterface<'a, C> + Copy,
{
    if arg.is_ignore() {
        return;
    }

    if !arg.layout.is_sized() {
        return;
    }

    if arg.layout.pass_indirectly_in_non_rustic_abis(cx) {
        arg.make_indirect();
        return;
    }

    // Patmos has no sub-word operations - extend all integers to 32-bit
    arg.extend_integer_width_to(32);

    let size_bytes = arg.layout.size.bytes();
    let words = ((size_bytes + 3) / 4) as usize;

    if arg.layout.is_aggregate() {
        // Patmos C ABI: aggregates >16 bytes, or those that don't fit in
        // remaining argument registers (r3-r8), are passed indirectly.
        if size_bytes > 16 || *state + words > 6 {
            arg.make_indirect();
        } else {
            // FIX: Box the CastTarget and supply missing fields
            arg.mode = PassMode::Cast {
                pad_i32: false,
                cast: Box::new(CastTarget {
                    prefix: Default::default(), // Handles empty prefix array types automatically
                    rest: Uniform {
                        unit: Reg::i32(),
                        total: arg.layout.size,
                        is_consecutive: false,
                    },
                    rest_offset: Some(Size::ZERO),
                    attrs: ArgAttributes::new(),
                }),
            };
            *state += words;
        }
    } else {
        // Scalars: use registers r3-r8, then spill to stack
        arg.mode = PassMode::Direct(ArgAttributes::new());
        if *state + words <= 6 {
            *state += words;
        } else {
            *state = 6;
        }
    }
}

pub(crate) fn compute_abi_info<'a, Ty, C>(cx: &C, fn_abi: &mut FnAbi<'a, Ty>)
where
    Ty: TyAbiInterface<'a, C> + Copy,
    C: HasDataLayout,
{
    let mut reg_state = 0;

    // Return values use r1/r2; large aggregates returned indirectly.
    if !fn_abi.ret.is_ignore() {
        fn_abi.ret.extend_integer_width_to(32);

        let ret_size = fn_abi.ret.layout.size.bytes();

        // Large aggregates (> 8 bytes) returned via pointer in r1
        // Small aggregates and scalars returned directly in r1 (and r2 if > 4 bytes)
        if fn_abi.ret.layout.is_aggregate() && ret_size > 8 {
            fn_abi.ret.make_indirect();
            reg_state += 1; // Pointer passed in r1
        } else if fn_abi.ret.layout.is_aggregate() {
            // FIX: Box the CastTarget and supply missing fields
            fn_abi.ret.mode = PassMode::Cast {
                pad_i32: false,
                cast: Box::new(CastTarget {
                    prefix: Default::default(),
                    rest: Uniform {
                        unit: Reg::i32(),
                        total: fn_abi.ret.layout.size,
                        is_consecutive: false,
                    },
                    rest_offset: Some(Size::ZERO),
                    attrs: ArgAttributes::new(),
                }),
            };
            let ret_words = ((ret_size + 3) / 4) as usize;
            reg_state = ret_words.min(2);
        } else {
            fn_abi.ret.mode = PassMode::Direct(ArgAttributes::new());
            let ret_words = ((ret_size + 3) / 4) as usize;
            reg_state = ret_words.min(2); // Max 2 registers (r1, r2)
        }
    }

    for arg in fn_abi.args.iter_mut() {
        classify_arg(cx, arg, &mut reg_state);
    }
}
