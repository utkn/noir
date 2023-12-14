
#define AvmMini_DECLARE_VIEWS(index)                                                                                   \
    using Accumulator = typename std::tuple_element<index, ContainerOverSubrelations>::type;                           \
    using View = typename Accumulator::View;                                                                           \
    [[maybe_unused]] auto avmMini_clk = View(new_term.avmMini_clk);                                                    \
    [[maybe_unused]] auto avmMini_first = View(new_term.avmMini_first);                                                \
    [[maybe_unused]] auto memTrace_m_clk = View(new_term.memTrace_m_clk);                                              \
    [[maybe_unused]] auto memTrace_m_sub_clk = View(new_term.memTrace_m_sub_clk);                                      \
    [[maybe_unused]] auto memTrace_m_addr = View(new_term.memTrace_m_addr);                                            \
    [[maybe_unused]] auto memTrace_m_val = View(new_term.memTrace_m_val);                                              \
    [[maybe_unused]] auto memTrace_m_lastAccess = View(new_term.memTrace_m_lastAccess);                                \
    [[maybe_unused]] auto memTrace_m_rw = View(new_term.memTrace_m_rw);                                                \
    [[maybe_unused]] auto avmMini_sel_op_add = View(new_term.avmMini_sel_op_add);                                      \
    [[maybe_unused]] auto avmMini_sel_op_sub = View(new_term.avmMini_sel_op_sub);                                      \
    [[maybe_unused]] auto avmMini_sel_op_mul = View(new_term.avmMini_sel_op_mul);                                      \
    [[maybe_unused]] auto avmMini_sel_op_div = View(new_term.avmMini_sel_op_div);                                      \
    [[maybe_unused]] auto avmMini_op_err = View(new_term.avmMini_op_err);                                              \
    [[maybe_unused]] auto avmMini_inv = View(new_term.avmMini_inv);                                                    \
    [[maybe_unused]] auto avmMini_ia = View(new_term.avmMini_ia);                                                      \
    [[maybe_unused]] auto avmMini_ib = View(new_term.avmMini_ib);                                                      \
    [[maybe_unused]] auto avmMini_ic = View(new_term.avmMini_ic);                                                      \
    [[maybe_unused]] auto avmMini_mem_op_a = View(new_term.avmMini_mem_op_a);                                          \
    [[maybe_unused]] auto avmMini_mem_op_b = View(new_term.avmMini_mem_op_b);                                          \
    [[maybe_unused]] auto avmMini_mem_op_c = View(new_term.avmMini_mem_op_c);                                          \
    [[maybe_unused]] auto avmMini_rwa = View(new_term.avmMini_rwa);                                                    \
    [[maybe_unused]] auto avmMini_rwb = View(new_term.avmMini_rwb);                                                    \
    [[maybe_unused]] auto avmMini_rwc = View(new_term.avmMini_rwc);                                                    \
    [[maybe_unused]] auto avmMini_mem_idx_a = View(new_term.avmMini_mem_idx_a);                                        \
    [[maybe_unused]] auto avmMini_mem_idx_b = View(new_term.avmMini_mem_idx_b);                                        \
    [[maybe_unused]] auto avmMini_mem_idx_c = View(new_term.avmMini_mem_idx_c);                                        \
    [[maybe_unused]] auto avmMini_last = View(new_term.avmMini_last);                                                  \
    [[maybe_unused]] auto memTrace_m_rw_shift = View(new_term.memTrace_m_rw_shift);                                    \
    [[maybe_unused]] auto memTrace_m_addr_shift = View(new_term.memTrace_m_addr_shift);                                \
    [[maybe_unused]] auto memTrace_m_val_shift = View(new_term.memTrace_m_val_shift);
