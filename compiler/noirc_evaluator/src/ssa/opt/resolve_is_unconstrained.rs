use crate::ssa::{
    ir::{
        function::{Function, RuntimeType},
        instruction::{Instruction, Intrinsic},
        types::NumericType,
        value::Value,
    },
    ssa_gen::Ssa,
};
use fxhash::FxHashSet as HashSet;

impl Ssa {
    /// An SSA pass to find any calls to `Intrinsic::IsUnconstrained` and replacing any uses of the result of the intrinsic
    /// with the resolved boolean value.
    /// Note that this pass must run after the pass that does runtime separation, since in SSA generation an ACIR function can end up targeting brillig.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn resolve_is_unconstrained(mut self) -> Self {
        for func in self.functions.values_mut() {
            func.replace_is_unconstrained_result();
        }
        self
    }
}

impl Function {
    pub(crate) fn replace_is_unconstrained_result(&mut self) {
        let mut is_unconstrained_calls = HashSet::default();
        let mut blocks_with_is_unconstrained_calls = HashSet::default();

        // Collect all calls to is_unconstrained
        for block_id in self.reachable_blocks() {
            for &instruction_id in self.dfg[block_id].instructions() {
                let target_func = match &self.dfg[instruction_id] {
                    Instruction::Call { func, .. } => *func,
                    _ => continue,
                };

                if let Value::Intrinsic(Intrinsic::IsUnconstrained) = self.dfg.resolve(target_func)
                {
                    is_unconstrained_calls.insert(instruction_id);
                    blocks_with_is_unconstrained_calls.insert(block_id);
                }
            }
        }

        for instruction_id in &is_unconstrained_calls {
            let original_return_id = Value::instruction_result(*instruction_id, 0);

            let is_unconstrained = matches!(self.runtime(), RuntimeType::Brillig(_)).into();
            let is_within_unconstrained = Value::constant(is_unconstrained, NumericType::bool());
            // Replace all uses of the original return value with the constant
            self.dfg.replace_value(original_return_id, is_within_unconstrained);
        }

        // Manually remove each call instruction that we just mapped to a constant
        for block in blocks_with_is_unconstrained_calls {
            self.dfg[block].instructions_mut().retain(|id| !is_unconstrained_calls.contains(id));
        }
    }
}
