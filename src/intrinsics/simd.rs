use crate::prelude::*;
use super::*;

pub fn codegen_simd_intrinsic_call<'tcx>(
    fx: &mut FunctionCx<'_, 'tcx, impl Backend>,
    instance: Instance<'tcx>,
    args: &[mir::Operand<'tcx>],
    ret: CPlace<'tcx>,
    span: Span,
) {
    let def_id = instance.def_id();
    let substs = instance.substs;

    let intrinsic = fx.tcx.item_name(def_id).as_str();
    let intrinsic = &intrinsic[..];

    intrinsic_match! {
        fx, intrinsic, substs, args,
        _ => {
            fx.tcx.sess.fatal(&format!("Unknown SIMD intrinsic {}", intrinsic));
        };

        simd_cast, (c a) {
            let (lane_layout, lane_count) = lane_type_and_count(fx, a.layout(), intrinsic);
            let (ret_lane_layout, ret_lane_count) = lane_type_and_count(fx, ret.layout(), intrinsic);
            assert_eq!(lane_count, ret_lane_count);

            let ret_lane_ty = fx.clif_type(ret_lane_layout.ty).unwrap();

            let from_signed = type_sign(lane_layout.ty);
            let to_signed = type_sign(ret_lane_layout.ty);

            for lane in 0..lane_count {
                let lane = mir::Field::new(lane.try_into().unwrap());

                let a_lane = a.value_field(fx, lane).load_scalar(fx);
                let res = clif_int_or_float_cast(fx, a_lane, from_signed, ret_lane_ty, to_signed);
                ret.place_field(fx, lane).write_cvalue(fx, CValue::by_val(res, ret_lane_layout));
            }
        };

        simd_eq, (c x, c y) {
            simd_cmp!(fx, intrinsic, Equal(x, y) -> ret);
        };
        simd_ne, (c x, c y) {
            simd_cmp!(fx, intrinsic, NotEqual(x, y) -> ret);
        };
        simd_lt, (c x, c y) {
            simd_cmp!(fx, intrinsic, UnsignedLessThan|SignedLessThan(x, y) -> ret);
        };
        simd_le, (c x, c y) {
            simd_cmp!(fx, intrinsic, UnsignedLessThanOrEqual|SignedLessThanOrEqual(x, y) -> ret);
        };
        simd_gt, (c x, c y) {
            simd_cmp!(fx, intrinsic, UnsignedGreaterThan|SignedGreaterThan(x, y) -> ret);
        };
        simd_ge, (c x, c y) {
            simd_cmp!(fx, intrinsic, UnsignedGreaterThanOrEqual|SignedGreaterThanOrEqual(x, y) -> ret);
        };

        // simd_shuffle32<T, U>(x: T, y: T, idx: [u32; 32]) -> U
        _ if intrinsic.starts_with("simd_shuffle"), (c x, c y, o idx) {
            let n: u32 = intrinsic["simd_shuffle".len()..].parse().unwrap();

            assert_eq!(x.layout(), y.layout());
            let layout = x.layout();

            let (lane_type, lane_count) = lane_type_and_count(fx, layout, intrinsic);
            let (ret_lane_type, ret_lane_count) = lane_type_and_count(fx, ret.layout(), intrinsic);

            assert_eq!(lane_type, ret_lane_type);
            assert_eq!(n, ret_lane_count);

            let total_len = lane_count * 2;

            let indexes = {
                use rustc::mir::interpret::*;
                let idx_const = crate::constant::mir_operand_get_const_val(fx, idx).expect("simd_shuffle* idx not const");

                let idx_bytes = match idx_const.val {
                    ty::ConstKind::Value(ConstValue::ByRef { alloc, offset }) => {
                        let ptr = Pointer::new(AllocId(0 /* dummy */), offset);
                        let size = Size::from_bytes(4 * u64::from(ret_lane_count) /* size_of([u32; ret_lane_count]) */);
                        alloc.get_bytes(fx, ptr, size).unwrap()
                    }
                    _ => unreachable!("{:?}", idx_const),
                };

                (0..ret_lane_count).map(|i| {
                    let i = usize::try_from(i).unwrap();
                    let idx = rustc::mir::interpret::read_target_uint(
                        fx.tcx.data_layout.endian,
                        &idx_bytes[4*i.. 4*i + 4],
                    ).expect("read_target_uint");
                    u32::try_from(idx).expect("try_from u32")
                }).collect::<Vec<u32>>()
            };

            for &idx in &indexes {
                assert!(idx < total_len, "idx {} out of range 0..{}", idx, total_len);
            }

            for (out_idx, in_idx) in indexes.into_iter().enumerate() {
                let in_lane = if in_idx < lane_count {
                    x.value_field(fx, mir::Field::new(in_idx.try_into().unwrap()))
                } else {
                    y.value_field(fx, mir::Field::new((in_idx - lane_count).try_into().unwrap()))
                };
                let out_lane = ret.place_field(fx, mir::Field::new(out_idx));
                out_lane.write_cvalue(fx, in_lane);
            }
        };

        simd_extract, (c v, o idx) {
            let idx_const = if let Some(idx_const) = crate::constant::mir_operand_get_const_val(fx, idx) {
                idx_const
            } else {
                fx.tcx.sess.span_warn(
                    fx.mir.span,
                    "`#[rustc_arg_required_const(..)]` is not yet supported. Calling this function will panic.",
                );
                crate::trap::trap_panic(fx, "`#[rustc_arg_required_const(..)]` is not yet supported.");
                return;
            };

            let idx = idx_const.val.try_to_bits(Size::from_bytes(4 /* u32*/)).expect(&format!("kind not scalar: {:?}", idx_const));
            let (_lane_type, lane_count) = lane_type_and_count(fx, v.layout(), intrinsic);
            if idx >= lane_count.into() {
                fx.tcx.sess.span_fatal(fx.mir.span, &format!("[simd_extract] idx {} >= lane_count {}", idx, lane_count));
            }

            let ret_lane = v.value_field(fx, mir::Field::new(idx.try_into().unwrap()));
            ret.write_cvalue(fx, ret_lane);
        };

        simd_add, (c x, c y) {
            simd_int_flt_binop!(fx, intrinsic, iadd|fadd(x, y) -> ret);
        };
        simd_sub, (c x, c y) {
            simd_int_flt_binop!(fx, intrinsic, isub|fsub(x, y) -> ret);
        };
        simd_mul, (c x, c y) {
            simd_int_flt_binop!(fx, intrinsic, imul|fmul(x, y) -> ret);
        };
        simd_div, (c x, c y) {
            simd_int_flt_binop!(fx, intrinsic, udiv|sdiv|fdiv(x, y) -> ret);
        };
        simd_shl, (c x, c y) {
            simd_int_binop!(fx, intrinsic, ishl(x, y) -> ret);
        };
        simd_shr, (c x, c y) {
            simd_int_binop!(fx, intrinsic, ushr|sshr(x, y) -> ret);
        };
        simd_and, (c x, c y) {
            simd_int_binop!(fx, intrinsic, band(x, y) -> ret);
        };
        simd_or, (c x, c y) {
            simd_int_binop!(fx, intrinsic, bor(x, y) -> ret);
        };
        simd_xor, (c x, c y) {
            simd_int_binop!(fx, intrinsic, bxor(x, y) -> ret);
        };

        simd_fmin, (c x, c y) {
            simd_flt_binop!(fx, intrinsic, fmin(x, y) -> ret);
        };
        simd_fmax, (c x, c y) {
            simd_flt_binop!(fx, intrinsic, fmax(x, y) -> ret);
        };
    }
}