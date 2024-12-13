use crate::ssa::ir::value::Value;
use super::InstructionId;

// The result of calling DataFlowGraph::insert_instruction can
// be a list of results or a single ValueId if the instruction was simplified
// to an existing value.
#[derive(Debug)]
pub(crate) enum InsertInstructionResult {
    Results { id: InstructionId, result_count: usize },
    SimplifiedTo(Value),
    SimplifiedToMultiple(Vec<Value>),
    InstructionRemoved,
}

impl InsertInstructionResult {
    /// Retrieve the first (and expected to be the only) result.
    pub(crate) fn first(&self) -> Value {
        match self {
            InsertInstructionResult::SimplifiedTo(value) => *value,
            InsertInstructionResult::SimplifiedToMultiple(values) => values[0],
            InsertInstructionResult::Results(instruction, results) => {
                assert_eq!(results.len(), 1);
                Value::Instruction { instruction: *instruction, position: 0 }
            }
            InsertInstructionResult::InstructionRemoved => {
                panic!("Instruction was removed, no results")
            }
        }
    }

    /// Return all the results contained in the internal results array.
    /// This is used for instructions returning multiple results like function calls.
    pub(crate) fn results(self) -> InsertInstructionResultIter {
        InsertInstructionResultIter { results: self, index: 0 }
    }

    /// Returns the amount of ValueIds contained
    pub(crate) fn len(&self) -> usize {
        match self {
            InsertInstructionResult::SimplifiedTo(_) => 1,
            InsertInstructionResult::SimplifiedToMultiple(results) => results.len(),
            InsertInstructionResult::Results(_, results) => results.len(),
            InsertInstructionResult::InstructionRemoved => 0,
        }
    }
}

pub(crate) struct InsertInstructionResultIter {
    results: InsertInstructionResult,
    index: usize,
}

impl Iterator for InsertInstructionResultIter {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        use InsertInstructionResult::*;
        match &self.results {
            Results { id, result_count } if self.index < *result_count => {
                let result = Value::Instruction { instruction: *id, position: self.index };
                self.index += 1;
                Some(result)
            },
            SimplifiedTo(value) if self.index == 0 => {
                self.index += 1;
                Some(value)
            },
            SimplifiedToMultiple(results) => {
                let result = results[self.index];
                self.index += 1;
                Some(result)
            },
            InstructionRemoved | Results { .. } | SimplifiedTo(..) => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let upper_bound = match &self.results {
            InsertInstructionResult::Results { result_count, .. } => *result_count,
            InsertInstructionResult::SimplifiedTo(_) => 1,
            InsertInstructionResult::SimplifiedToMultiple(results) => results.len(),
            InsertInstructionResult::InstructionRemoved => 0,
        };
        (0, Some(upper_bound))
    }
}

impl ExactSizeIterator for InsertInstructionResultIter {}

impl std::ops::Index<usize> for InsertInstructionResult {
    type Output = Value;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            InsertInstructionResult::Results(instruction, result_count) => {
                assert!(index < result_count);
                &Value::Instruction { instruction: *instruction, position: index }
            }
            InsertInstructionResult::SimplifiedTo(result) => {
                assert_eq!(index, 0);
                result
            }
            InsertInstructionResult::SimplifiedToMultiple(results) => &results[index],
            InsertInstructionResult::InstructionRemoved => {
                panic!("Cannot index into InsertInstructionResult::InstructionRemoved")
            }
        }
    }
}
