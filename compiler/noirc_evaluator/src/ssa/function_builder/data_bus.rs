use std::{collections::BTreeMap, sync::Arc};

use crate::ssa::ir::{function::RuntimeType, types::Type, value::Value};
use fxhash::FxHashMap as HashMap;
use iter_extended::vecmap;
use noirc_frontend::ast;
use noirc_frontend::hir_def::function::FunctionSignature;
use serde::{Deserialize, Serialize};

use super::FunctionBuilder;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatabusVisibility {
    None,
    CallData(u32),
    ReturnData,
}
/// Used to create a data bus, which is an array of private inputs
/// replacing public inputs
#[derive(Clone, Debug)]
pub(crate) struct DataBusBuilder {
    pub(crate) values: im::Vector<Value>,
    index: usize,
    pub(crate) map: HashMap<Value, usize>,
    pub(crate) databus: Option<Value>,
    call_data_id: Option<u32>,
}

impl DataBusBuilder {
    pub(crate) fn new() -> DataBusBuilder {
        DataBusBuilder {
            index: 0,
            map: HashMap::default(),
            databus: None,
            values: im::Vector::new(),
            call_data_id: None,
        }
    }

    /// Generates a vector telling which flattened parameters from the given function signature
    /// are tagged with databus visibility
    pub(crate) fn is_databus(main_signature: &FunctionSignature) -> Vec<DatabusVisibility> {
        let mut params_is_databus = Vec::new();

        for param in &main_signature.0 {
            let is_databus = match param.2 {
                ast::Visibility::Public | ast::Visibility::Private => DatabusVisibility::None,
                ast::Visibility::CallData(id) => DatabusVisibility::CallData(id),
                ast::Visibility::ReturnData => DatabusVisibility::ReturnData,
            };
            let len = param.1.field_count(&param.0.location()) as usize;
            params_is_databus.extend(vec![is_databus; len]);
        }
        params_is_databus
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CallData {
    /// The id to this calldata assigned by the user
    pub(crate) call_data_id: u32,
    pub(crate) array_id: Value,
    pub(crate) index_map: HashMap<Value, usize>,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub(crate) struct DataBus {
    pub(crate) call_data: Vec<CallData>,
    pub(crate) return_data: Option<Value>,
}

impl DataBus {
    /// Updates the databus values with the provided function
    pub(crate) fn map_values(&self, mut f: impl FnMut(Value) -> Value) -> DataBus {
        let call_data = self
            .call_data
            .iter()
            .map(|cd| {
                let mut call_data_map = HashMap::default();
                for (k, v) in cd.index_map.iter() {
                    call_data_map.insert(f(*k), *v);
                }
                CallData {
                    array_id: f(cd.array_id),
                    index_map: call_data_map,
                    call_data_id: cd.call_data_id,
                }
            })
            .collect();
        DataBus { call_data, return_data: self.return_data.map(&mut f) }
    }

    /// Updates the databus values in place with the provided function
    pub(crate) fn map_values_mut(&mut self, mut f: impl FnMut(Value) -> Value) {
        for cd in self.call_data.iter_mut() {
            cd.array_id = f(cd.array_id);

            // Can't mutate a hashmap's keys so we need to collect into a new one.
            cd.index_map = cd.index_map.iter().map(|(k, v)| (f(*k), *v)).collect();
        }

        if let Some(data) = self.return_data.as_mut() {
            *data = f(*data);
        }
    }

    pub(crate) fn call_data_array(&self) -> Vec<(u32, Value)> {
        self.call_data.iter().map(|cd| (cd.call_data_id, cd.array_id)).collect()
    }
    /// Construct a databus from call_data and return_data data bus builders
    pub(crate) fn get_data_bus(
        call_data: Vec<DataBusBuilder>,
        return_data: DataBusBuilder,
    ) -> DataBus {
        let mut call_data_args = Vec::new();
        for call_data_item in call_data {
            // databus can be None if `main` is a brillig function
            let Some(array_id) = call_data_item.databus else { continue };
            let call_data_id =
                call_data_item.call_data_id.expect("Call data should have a user id");
            call_data_args.push(CallData { array_id, call_data_id, index_map: call_data_item.map });
        }

        DataBus { call_data: call_data_args, return_data: return_data.databus }
    }
}

impl FunctionBuilder {
    /// Insert a value into a data bus builder
    fn add_to_data_bus(&mut self, value: Value, databus: &mut DataBusBuilder) {
        assert!(databus.databus.is_none(), "initializing finalized call data");
        let typ = self.current_function.dfg.type_of_value(value);
        match typ {
            Type::Numeric(_) => {
                databus.values.push_back(value);
                databus.index += 1;
            }
            Type::Array(typ, len) => {
                databus.map.insert(value, databus.index);

                let mut index = 0;
                for _i in 0..len {
                    for subitem_typ in typ.iter() {
                        // load each element of the array, and add it to the databus
                        let index_var =
                            self.current_function.dfg.length_constant((index as i128).into());
                        let element = self.insert_array_get(value, index_var, subitem_typ.clone());
                        index += match subitem_typ {
                            Type::Array(_, _) | Type::Slice(_) => subitem_typ.element_size(),
                            Type::Numeric(_) => 1,
                            _ => unreachable!("Unsupported type for databus"),
                        };
                        self.add_to_data_bus(element, databus);
                    }
                }
            }
            Type::Reference(_) => {
                unreachable!("Attempted to add invalid type (reference) to databus")
            }
            Type::Slice(_) => unreachable!("Attempted to add invalid type (slice) to databus"),
            Type::Function => unreachable!("Attempted to add invalid type (function) to databus"),
        }
    }

    /// Create a data bus builder from a list of values
    pub(crate) fn initialize_data_bus(
        &mut self,
        values: &[Value],
        mut databus: DataBusBuilder,
        call_data_id: Option<u32>,
    ) -> DataBusBuilder {
        for value in values {
            self.add_to_data_bus(*value, &mut databus);
        }
        let len = databus.values.len() as u32;

        let array = (len > 0 && matches!(self.current_function.runtime(), RuntimeType::Acir(_)))
            .then(|| {
                let array_type = Type::Array(Arc::new(vec![Type::field()]), len);
                self.insert_make_array(databus.values, array_type)
            });

        DataBusBuilder {
            index: 0,
            map: databus.map,
            databus: array,
            values: im::Vector::new(),
            call_data_id,
        }
    }

    /// Generate the data bus for call-data, based on the parameters of the entry block
    /// and a vector telling which ones are call-data
    pub(crate) fn call_data_bus(
        &mut self,
        flattened_databus_visibilities: Vec<DatabusVisibility>,
    ) -> Vec<DataBusBuilder> {
        //filter parameters of the first block that have call-data visibility
        let first_block = self.current_function.entry_block();
        let params = self.current_function.dfg.block_parameters(first_block);

        // Reshape the is_params_databus to map to the SSA-level parameters
        let is_params_databus =
            self.deflatten_databus_visibilities(params, flattened_databus_visibilities);

        let params = self.current_function.dfg.block_parameters(first_block);
        let mut databus_param: BTreeMap<u32, Vec<Value>> = BTreeMap::new();

        for (param, databus_attribute) in params.zip(is_params_databus) {
            match databus_attribute {
                DatabusVisibility::None | DatabusVisibility::ReturnData => continue,
                DatabusVisibility::CallData(call_data_id) => {
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        databus_param.entry(call_data_id)
                    {
                        e.insert(vec![param.to_owned()]);
                    } else {
                        databus_param.get_mut(&call_data_id).unwrap().push(param.to_owned());
                    }
                }
            }
        }
        // create the call-data-bus from the filtered lists
        vecmap(databus_param.keys(), |id| {
            self.initialize_data_bus(&databus_param[id], DataBusBuilder::new(), Some(*id))
        })
    }

    /// This function takes the flattened databus visibilities and generates the databus visibility for each ssa parameter
    /// asserting that an ssa parameter is not assigned two different databus visibilities
    fn deflatten_databus_visibilities(
        &self,
        ssa_params: impl ExactSizeIterator<Item = Value>,
        mut flattened_params_databus_visibility: Vec<DatabusVisibility>,
    ) -> Vec<DatabusVisibility> {
        let ssa_params_len = ssa_params.len();
        let ssa_param_sizes = ssa_params.map(|ssa_param| {
            self.current_function.dfg.type_of_value(ssa_param).flattened_size() as usize
        });

        let mut is_ssa_params_databus = Vec::with_capacity(ssa_param_sizes.len());
        for size in ssa_param_sizes {
            let visibilities: Vec<DatabusVisibility> =
                flattened_params_databus_visibility.drain(0..size).collect();
            let visibility = visibilities.first().copied().unwrap_or(DatabusVisibility::None);
            assert!(
                visibilities.iter().all(|v| *v == visibility),
                "inconsistent databus visibility for ssa param"
            );
            is_ssa_params_databus.push(visibility);
        }

        assert_eq!(is_ssa_params_databus.len(), ssa_params_len);
        is_ssa_params_databus
    }
}
