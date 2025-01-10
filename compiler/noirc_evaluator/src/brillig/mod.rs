pub(crate) mod brillig_gen;
pub(crate) mod brillig_ir;

use acvm::{acir::brillig::MemoryAddress, FieldElement};
use brillig_gen::brillig_block_variables::allocate_value_with_type;
use brillig_ir::{
    artifact::LabelType,
    brillig_variable::{BrilligVariable, SingleAddrVariable},
    registers::GlobalSpace,
    BrilligBinaryOp, BrilligContext, ReservedRegisters,
};

use self::{
    brillig_gen::convert_ssa_function,
    brillig_ir::{
        artifact::{BrilligArtifact, Label},
        procedures::compile_procedure,
    },
};
use crate::ssa::{
    ir::{
        dfg::DataFlowGraph,
        function::{Function, FunctionId},
        instruction::Instruction,
        types::Type,
        value::{Value, ValueId},
    },
    ssa_gen::Ssa,
};
use fxhash::FxHashMap as HashMap;
use std::{borrow::Cow, collections::BTreeSet, sync::Arc};

pub use self::brillig_ir::procedures::ProcedureId;

/// Context structure for the brillig pass.
/// It stores brillig-related data required for brillig generation.
#[derive(Default)]
pub struct Brillig {
    /// Maps SSA function labels to their brillig artifact
    ssa_function_to_brillig: HashMap<FunctionId, BrilligArtifact<FieldElement>>,
    globals: BrilligArtifact<FieldElement>,
}

impl Brillig {
    /// Compiles a function into brillig and store the compilation artifacts
    pub(crate) fn compile(
        &mut self,
        func: &Function,
        enable_debug_trace: bool,
        globals: &HashMap<ValueId, BrilligVariable>,
    ) {
        let obj = convert_ssa_function(func, enable_debug_trace, globals);
        self.ssa_function_to_brillig.insert(func.id(), obj);
    }

    /// Finds a brillig artifact by its label
    pub(crate) fn find_by_label(
        &self,
        function_label: Label,
    ) -> Option<Cow<BrilligArtifact<FieldElement>>> {
        match function_label.label_type {
            LabelType::Function(function_id, _) => {
                self.ssa_function_to_brillig.get(&function_id).map(Cow::Borrowed)
            }
            // Procedures are compiled as needed
            LabelType::Procedure(procedure_id) => Some(Cow::Owned(compile_procedure(procedure_id))),
            LabelType::GlobalInit => Some(Cow::Borrowed(&self.globals)),
            _ => unreachable!("ICE: Expected a function or procedure label"),
        }
    }

    pub(crate) fn create_brillig_globals(
        brillig_context: &mut BrilligContext<FieldElement, GlobalSpace>,
        globals: &DataFlowGraph,
    ) -> HashMap<ValueId, BrilligVariable> {
        let mut brillig_globals = HashMap::default();
        for (id, value) in globals.values_iter() {
            match value {
                Value::NumericConstant { constant, typ } => {
                    let new_variable =
                        allocate_value_with_type(brillig_context, Type::Numeric(*typ));
                    dbg!(new_variable.clone());
                    brillig_context
                        .const_instruction(new_variable.extract_single_addr(), *constant);

                    brillig_globals.insert(id, new_variable);
                }
                Value::Instruction { instruction, .. } => {
                    let result = globals.instruction_results(*instruction)[0];
                    dbg!(result);
                    let instruction = &globals[*instruction];
                    match &instruction {
                        Instruction::MakeArray { elements: array, typ } => {
                            let new_variable =
                                allocate_value_with_type(brillig_context, typ.clone());
                            // Initialize the variable
                            match new_variable {
                                BrilligVariable::BrilligArray(brillig_array) => {
                                    brillig_context.codegen_initialize_array(brillig_array);
                                }
                                BrilligVariable::BrilligVector(vector) => {
                                    let size = brillig_context
                                        .make_usize_constant_instruction(array.len().into());
                                    brillig_context.codegen_initialize_vector(vector, size, None);
                                    brillig_context.deallocate_single_addr(size);
                                }
                                _ => unreachable!(
                                    "ICE: Cannot initialize array value created as {new_variable:?}"
                                ),
                            };

                            // Write the items
                            let items_pointer = brillig_context
                                .codegen_make_array_or_vector_items_pointer(new_variable);

                            Self::initialize_constant_array(
                                array,
                                typ,
                                items_pointer,
                                brillig_context,
                                &brillig_globals,
                            );

                            brillig_context.deallocate_register(items_pointer);

                            dbg!(new_variable.clone());
                            brillig_globals.insert(result, new_variable);
                        }
                        _ => {
                            unreachable!("Expected MakeArray instruction but got {instruction:#?}")
                        }
                    }
                }
                _ => {
                    panic!("got something other than numeric constant")
                }
            }
        }
        brillig_globals
    }

    fn initialize_constant_array(
        data: &im::Vector<ValueId>,
        typ: &Type,
        pointer: MemoryAddress,
        brillig_context: &mut BrilligContext<FieldElement, GlobalSpace>,
        brillig_globals: &HashMap<ValueId, BrilligVariable>,
    ) {
        if data.is_empty() {
            return;
        }
        let item_types = typ.clone().element_types();

        // Find out if we are repeating the same item over and over
        let first_item = data.iter().take(item_types.len()).copied().collect();
        let mut is_repeating = true;

        for item_index in (item_types.len()..data.len()).step_by(item_types.len()) {
            let item: Vec<_> = (0..item_types.len()).map(|i| data[item_index + i]).collect();
            if first_item != item {
                is_repeating = false;
                break;
            }
        }

        // If all the items are single address, and all have the same initial value, we can initialize the array in a runtime loop.
        // Since the cost in instructions for a runtime loop is in the order of magnitude of 10, we only do this if the item_count is bigger than that.
        let item_count = data.len() / item_types.len();

        if item_count > 10
            && is_repeating
            && item_types.iter().all(|typ| matches!(typ, Type::Numeric(_)))
        {
            dbg!("initializing runtime");
            Self::initialize_constant_array_runtime(
                item_types,
                first_item,
                item_count,
                pointer,
                brillig_context,
                &brillig_globals,
            );
        } else {
            dbg!("initializing comptime");
            Self::initialize_constant_array_comptime(
                data,
                pointer,
                brillig_context,
                &brillig_globals,
            );
        }
    }

    fn initialize_constant_array_runtime(
        item_types: Arc<Vec<Type>>,
        item_to_repeat: Vec<ValueId>,
        item_count: usize,
        pointer: MemoryAddress,
        brillig_context: &mut BrilligContext<FieldElement, GlobalSpace>,
        brillig_globals: &HashMap<ValueId, BrilligVariable>,
    ) {
        let mut subitem_to_repeat_variables = Vec::with_capacity(item_types.len());
        for subitem_id in item_to_repeat.into_iter() {
            subitem_to_repeat_variables.push(
                *brillig_globals
                    .get(&subitem_id)
                    .unwrap_or_else(|| panic!("ICE: ValueId {subitem_id} is not available")),
            );
        }

        // Initialize loop bound with the array length
        let end_pointer_variable =
            brillig_context.make_usize_constant_instruction((item_count * item_types.len()).into());

        // Add the pointer to the array length
        brillig_context.memory_op_instruction(
            end_pointer_variable.address,
            pointer,
            end_pointer_variable.address,
            BrilligBinaryOp::Add,
        );

        // If this is an array with complex subitems, we need a custom step in the loop to write all the subitems while iterating.
        if item_types.len() > 1 {
            let step_variable =
                brillig_context.make_usize_constant_instruction(item_types.len().into());

            let subitem_pointer =
                SingleAddrVariable::new_usize(brillig_context.allocate_register());

            // Initializes a single subitem
            let initializer_fn =
                |ctx: &mut BrilligContext<_, _>, subitem_start_pointer: SingleAddrVariable| {
                    ctx.mov_instruction(subitem_pointer.address, subitem_start_pointer.address);
                    for (subitem_index, subitem) in
                        subitem_to_repeat_variables.into_iter().enumerate()
                    {
                        ctx.store_instruction(subitem_pointer.address, subitem.extract_register());
                        if subitem_index != item_types.len() - 1 {
                            ctx.memory_op_instruction(
                                subitem_pointer.address,
                                ReservedRegisters::usize_one(),
                                subitem_pointer.address,
                                BrilligBinaryOp::Add,
                            );
                        }
                    }
                };

            // for (let subitem_start_pointer = pointer; subitem_start_pointer < pointer + data_length; subitem_start_pointer += step) { initializer_fn(iterator) }
            brillig_context.codegen_for_loop(
                Some(pointer),
                end_pointer_variable.address,
                Some(step_variable.address),
                initializer_fn,
            );

            brillig_context.deallocate_single_addr(step_variable);
            brillig_context.deallocate_single_addr(subitem_pointer);
        } else {
            let subitem = subitem_to_repeat_variables.into_iter().next().unwrap();

            let initializer_fn =
                |ctx: &mut BrilligContext<_, _>, item_pointer: SingleAddrVariable| {
                    ctx.store_instruction(item_pointer.address, subitem.extract_register());
                };

            // for (let item_pointer = pointer; item_pointer < pointer + data_length; item_pointer += 1) { initializer_fn(iterator) }
            brillig_context.codegen_for_loop(
                Some(pointer),
                end_pointer_variable.address,
                None,
                initializer_fn,
            );
        }
        brillig_context.deallocate_single_addr(end_pointer_variable);
    }

    fn initialize_constant_array_comptime(
        data: &im::Vector<crate::ssa::ir::map::Id<Value>>,
        pointer: MemoryAddress,
        brillig_context: &mut BrilligContext<FieldElement, GlobalSpace>,
        brillig_globals: &HashMap<ValueId, BrilligVariable>,
    ) {
        // Allocate a register for the iterator
        let write_pointer_register = brillig_context.allocate_register();

        brillig_context.mov_instruction(write_pointer_register, pointer);

        for (element_idx, element_id) in data.iter().enumerate() {
            let element_variable = *brillig_globals
                .get(&element_id)
                .unwrap_or_else(|| panic!("ICE: ValueId {element_id} is not available"));
            // Store the item in memory
            brillig_context
                .store_instruction(write_pointer_register, element_variable.extract_register());

            if element_idx != data.len() - 1 {
                // Increment the write_pointer_register
                brillig_context.memory_op_instruction(
                    write_pointer_register,
                    ReservedRegisters::usize_one(),
                    write_pointer_register,
                    BrilligBinaryOp::Add,
                );
            }
        }

        brillig_context.deallocate_register(write_pointer_register);
    }
}

impl std::ops::Index<FunctionId> for Brillig {
    type Output = BrilligArtifact<FieldElement>;
    fn index(&self, id: FunctionId) -> &Self::Output {
        &self.ssa_function_to_brillig[&id]
    }
}

impl Ssa {
    /// Compile Brillig functions and ACIR functions reachable from them
    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn to_brillig(&self, enable_debug_trace: bool) -> Brillig {
        // Collect all the function ids that are reachable from brillig
        // That means all the functions marked as brillig and ACIR functions called by them
        let brillig_reachable_function_ids = self
            .functions
            .iter()
            .filter_map(|(id, func)| func.runtime().is_brillig().then_some(*id))
            .collect::<BTreeSet<_>>();

        let mut brillig = Brillig::default();

        let mut brillig_context = BrilligContext::new_for_global_init(enable_debug_trace);
        brillig_context.enter_context(Label::globals_init());
        let brillig_globals = Brillig::create_brillig_globals(&mut brillig_context, &self.globals);
        brillig_context.return_instruction();

        let artifact = brillig_context.artifact();
        brillig.globals = artifact;

        for brillig_function_id in brillig_reachable_function_ids {
            let func = &self.functions[&brillig_function_id];
            brillig.compile(func, enable_debug_trace, &brillig_globals);
        }

        brillig
    }
}
