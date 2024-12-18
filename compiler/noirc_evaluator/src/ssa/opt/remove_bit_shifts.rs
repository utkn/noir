use acvm::{acir::AcirField, FieldElement};
use std::sync::Arc;

use crate::ssa::{
    ir::{
        basic_block::BasicBlockId,
        call_stack::CallStackId,
        function::{Function, RuntimeType},
        instruction::insert_result::InsertInstructionResult,
        instruction::{
            insert_result::InsertInstructionResultIter, Binary, BinaryOp, Endian, Instruction,
            InstructionId, Intrinsic,
        },
        types::{NumericType, Type},
        value::Value,
    },
    ssa_gen::Ssa,
};

impl Ssa {
    /// Performs constant folding on each instruction.
    ///
    /// See [`constant_folding`][self] module for more information.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn remove_bit_shifts(mut self) -> Ssa {
        for function in self.functions.values_mut() {
            function.remove_bit_shifts();
        }
        self
    }
}

impl Function {
    /// The structure of this pass is simple:
    /// Go through each block and re-insert all instructions.
    pub(crate) fn remove_bit_shifts(&mut self) {
        if matches!(self.runtime(), RuntimeType::Brillig(_)) {
            return;
        }

        let block = self.entry_block();
        let mut context = Context {
            function: self,
            new_instructions: Vec::new(),
            block,
            call_stack: CallStackId::root(),
        };

        context.remove_bit_shifts();
    }
}

struct Context<'f> {
    function: &'f mut Function,
    new_instructions: Vec<InstructionId>,

    block: BasicBlockId,
    call_stack: CallStackId,
}

impl Context<'_> {
    fn remove_bit_shifts(&mut self) {
        let instructions = self.function.dfg[self.block].take_instructions();

        for instruction_id in instructions {
            match self.function.dfg[instruction_id] {
                Instruction::Binary(Binary { lhs, rhs, operator })
                    if matches!(operator, BinaryOp::Shl | BinaryOp::Shr) =>
                {
                    self.call_stack =
                        self.function.dfg.get_instruction_call_stack_id(instruction_id);
                    let old_result = Value::instruction_result(instruction_id, 0);

                    let bit_size = match self.function.dfg.type_of_value(lhs) {
                        Type::Numeric(NumericType::Signed { bit_size })
                        | Type::Numeric(NumericType::Unsigned { bit_size }) => bit_size,
                        _ => unreachable!("ICE: right-shift attempted on non-integer"),
                    };
                    let new_result = if operator == BinaryOp::Shl {
                        self.insert_wrapping_shift_left(lhs, rhs, bit_size)
                    } else {
                        self.insert_shift_right(lhs, rhs, bit_size)
                    };

                    self.function.dfg.replace_value(old_result, new_result);
                }
                _ => {
                    self.new_instructions.push(instruction_id);
                }
            };
        }

        *self.function.dfg[self.block].instructions_mut() =
            std::mem::take(&mut self.new_instructions);
    }

    /// Insert ssa instructions which computes lhs << rhs by doing lhs*2^rhs
    /// and truncate the result to bit_size
    pub(crate) fn insert_wrapping_shift_left(
        &mut self,
        lhs: Value,
        rhs: Value,
        bit_size: u8,
    ) -> Value {
        let base = self.function.dfg.field_constant(FieldElement::from(2_u128));
        let typ = self.function.dfg.type_of_value(lhs).unwrap_numeric();
        let (max_bit, pow) = if let Some(rhs_constant) = self.function.dfg.get_numeric_constant(rhs)
        {
            // Happy case is that we know precisely by how many bits the integer will
            // increase: lhs_bit_size + rhs
            let bit_shift_size = rhs_constant.to_u128() as u32;

            let (rhs_bit_size_pow_2, overflows) = 2_u128.overflowing_pow(bit_shift_size);
            if overflows {
                assert!(bit_size < 128, "ICE - shift left with big integers are not supported");
                if bit_size < 128 {
                    return self.function.dfg.constant(FieldElement::zero(), typ);
                }
            }
            let pow = self.function.dfg.constant(FieldElement::from(rhs_bit_size_pow_2), typ);

            let max_lhs_bits = self.function.dfg.get_value_max_num_bits(lhs);

            (max_lhs_bits + bit_shift_size as u8, pow)
        } else {
            // we use a predicate to nullify the result in case of overflow
            let u8_type = NumericType::unsigned(8);
            let bit_size_var =
                self.function.dfg.constant(FieldElement::from(bit_size as u128), u8_type);
            let overflow = self.insert_binary(rhs, BinaryOp::Lt, bit_size_var);
            let predicate = self.insert_cast(overflow, typ);
            // we can safely cast to unsigned because overflow_checks prevent bit-shift with a negative value
            let rhs_unsigned = self.insert_cast(rhs, NumericType::unsigned(bit_size));
            let pow = self.pow(base, rhs_unsigned);
            let pow = self.insert_cast(pow, typ);
            (
                FieldElement::max_num_bits().try_into().unwrap(),
                self.insert_binary(predicate, BinaryOp::Mul, pow),
            )
        };

        if max_bit <= bit_size {
            self.insert_binary(lhs, BinaryOp::Mul, pow)
        } else {
            let lhs_field = self.insert_cast(lhs, NumericType::NativeField);
            let pow_field = self.insert_cast(pow, NumericType::NativeField);
            let result = self.insert_binary(lhs_field, BinaryOp::Mul, pow_field);
            let result = self.insert_truncate(result, bit_size, max_bit);
            self.insert_cast(result, typ)
        }
    }

    /// Insert ssa instructions which computes lhs >> rhs by doing lhs/2^rhs
    /// For negative signed integers, we do the division on the 1-complement representation of lhs,
    /// before converting back the result to the 2-complement representation.
    pub(crate) fn insert_shift_right(&mut self, lhs: Value, rhs: Value, bit_size: u8) -> Value {
        let lhs_typ = self.function.dfg.type_of_value(lhs).unwrap_numeric();
        let base = self.function.dfg.field_constant(FieldElement::from(2_u128));
        let pow = self.pow(base, rhs);
        if lhs_typ.is_unsigned() {
            // unsigned right bit shift is just a normal division
            self.insert_binary(lhs, BinaryOp::Div, pow)
        } else {
            // Get the sign of the operand; positive signed operand will just do a division as well
            let zero =
                self.function.dfg.constant(FieldElement::zero(), NumericType::signed(bit_size));
            let lhs_sign = self.insert_binary(lhs, BinaryOp::Lt, zero);
            let lhs_sign_as_field = self.insert_cast(lhs_sign, NumericType::NativeField);
            let lhs_as_field = self.insert_cast(lhs, NumericType::NativeField);
            // For negative numbers, convert to 1-complement using wrapping addition of a + 1
            let one_complement = self.insert_binary(lhs_sign_as_field, BinaryOp::Add, lhs_as_field);
            let one_complement = self.insert_truncate(one_complement, bit_size, bit_size + 1);
            let one_complement = self.insert_cast(one_complement, NumericType::signed(bit_size));
            // Performs the division on the 1-complement (or the operand if positive)
            let shifted_complement = self.insert_binary(one_complement, BinaryOp::Div, pow);
            // Convert back to 2-complement representation if operand is negative
            let lhs_sign_as_int = self.insert_cast(lhs_sign, lhs_typ);
            let shifted = self.insert_binary(shifted_complement, BinaryOp::Sub, lhs_sign_as_int);
            self.insert_truncate(shifted, bit_size, bit_size + 1)
        }
    }

    /// Computes lhs^rhs via square&multiply, using the bits decomposition of rhs
    /// Pseudo-code of the computation:
    /// let mut r = 1;
    /// let rhs_bits = to_bits(rhs);
    /// for i in 1 .. bit_size + 1 {
    ///     let r_squared = r * r;
    ///     let b = rhs_bits[bit_size - i];
    ///     r = (r_squared * lhs * b) + (1 - b) * r_squared;
    /// }
    fn pow(&mut self, lhs: Value, rhs: Value) -> Value {
        let typ = self.function.dfg.type_of_value(rhs);
        if let Type::Numeric(NumericType::Unsigned { bit_size }) = typ {
            let to_bits = Value::Intrinsic(Intrinsic::ToBits(Endian::Little));
            let result_types = vec![Type::Array(Arc::new(vec![Type::bool()]), bit_size as u32)];

            let rhs_bits = self.insert_call(to_bits, vec![rhs], result_types).next().unwrap();

            let one = self.function.dfg.field_constant(FieldElement::one());
            let mut r = one;
            for i in 1..bit_size + 1 {
                let r_squared = self.insert_binary(r, BinaryOp::Mul, r);
                let a = self.insert_binary(r_squared, BinaryOp::Mul, lhs);
                let idx =
                    self.function.dfg.field_constant(FieldElement::from((bit_size - i) as i128));
                let b = self.insert_array_get(rhs_bits, idx, Type::bool());
                let not_b = self.insert_not(b);
                let b = self.insert_cast(b, NumericType::NativeField);
                let not_b = self.insert_cast(not_b, NumericType::NativeField);
                let r1 = self.insert_binary(a, BinaryOp::Mul, b);
                let r2 = self.insert_binary(r_squared, BinaryOp::Mul, not_b);
                r = self.insert_binary(r1, BinaryOp::Add, r2);
            }
            r
        } else {
            unreachable!("Value must be unsigned in power operation");
        }
    }

    /// Insert a binary instruction at the end of the current block.
    /// Returns the result of the binary instruction.
    pub(crate) fn insert_binary(&mut self, lhs: Value, operator: BinaryOp, rhs: Value) -> Value {
        let instruction = Instruction::Binary(Binary { lhs, rhs, operator });
        self.insert_instruction(instruction).first()
    }

    /// Insert a not instruction at the end of the current block.
    /// Returns the result of the instruction.
    pub(crate) fn insert_not(&mut self, rhs: Value) -> Value {
        self.insert_instruction(Instruction::Not(rhs)).first()
    }

    /// Insert a truncate instruction at the end of the current block.
    /// Returns the result of the truncate instruction.
    pub(crate) fn insert_truncate(
        &mut self,
        value: Value,
        bit_size: u8,
        max_bit_size: u8,
    ) -> Value {
        self.insert_instruction(Instruction::Truncate { value, bit_size, max_bit_size }).first()
    }

    /// Insert a cast instruction at the end of the current block.
    /// Returns the result of the cast instruction.
    pub(crate) fn insert_cast(&mut self, value: Value, typ: NumericType) -> Value {
        self.insert_instruction(Instruction::Cast(value, typ)).first()
    }

    /// Insert a call instruction at the end of the current block and return
    /// the results of the call.
    pub(crate) fn insert_call(
        &mut self,
        func: Value,
        arguments: Vec<Value>,
        result_types: Vec<Type>,
    ) -> InsertInstructionResultIter {
        let call = Instruction::Call { func, arguments, result_types };
        self.insert_instruction(call).results()
    }

    /// Insert an instruction to extract an element from an array
    pub(crate) fn insert_array_get(
        &mut self,
        array: Value,
        index: Value,
        result_type: Type,
    ) -> Value {
        self.insert_instruction(Instruction::ArrayGet { array, index, result_type }).first()
    }

    pub(crate) fn insert_instruction(
        &mut self,
        instruction: Instruction,
    ) -> InsertInstructionResult {
        let result = self.function.dfg.insert_instruction_and_results(
            instruction,
            self.block,
            self.call_stack,
        );

        if let InsertInstructionResult::Results { id, .. } = result {
            self.new_instructions.push(id);
        }

        result
    }
}
