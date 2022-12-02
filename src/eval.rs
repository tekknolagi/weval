//! Partial evaluation.

/* TODO:
- inlining
- "memory renaming": connecting symbolic ops through the operand-stack
  memory region
- more general memory-region handling: symbolic but unique
  (non-escaped) pointers, stack, operand-stack region, ...
*/

use crate::directive::Directive;
use crate::image::Image;
use crate::intrinsics::Intrinsics;
use crate::state::*;
use crate::value::{AbstractValue, ValueTags, WasmVal};
use std::borrow::Cow;
use std::collections::{
    btree_map::Entry as BTreeEntry, hash_map::Entry as HashEntry, HashMap, HashSet, VecDeque,
};
use waffle::cfg::CFGInfo;
use waffle::{
    entity::EntityRef, Block, BlockTarget, FunctionBody, Module, Operator, Terminator, Type, Value,
    ValueDef,
};

struct Evaluator<'a> {
    /// Original function body.
    generic: &'a FunctionBody,
    /// Intrinsic function indices.
    intrinsics: &'a Intrinsics,
    /// Memory image.
    image: &'a Image,
    /// Domtree for function body.
    cfg: CFGInfo,
    /// State of SSA values and program points:
    /// - per context:
    ///   - per SSA number, an abstract value
    ///   - per block, entry state for that block
    state: FunctionState,
    /// New function body.
    func: FunctionBody,
    /// Map of (ctx, block_in_generic) to specialized block_in_func.
    block_map: HashMap<(Context, Block), Block>,
    /// Dependencies for updates: some use in a given block with a
    /// given context occurs of a value defined in another block at
    /// another context.
    block_deps: HashMap<(Context, Block), HashSet<(Context, Block)>>,
    /// Map of (ctx, value_in_generic) to specialized value_in_func.
    value_map: HashMap<(Context, Value), Value>,
    /// Queue of blocks to (re)compute. List of (block_in_generic,
    /// ctx, block_in_func).
    queue: VecDeque<(Block, Context, Block)>,
    /// Set to deduplicate `queue`.
    queue_set: HashSet<(Block, Context)>,
    /// Header blocks.
    header_blocks: HashSet<Block>,
}

/// Partially evaluates according to the given directives.
pub fn partially_evaluate(
    module: &mut Module,
    im: &mut Image,
    directives: &[Directive],
) -> anyhow::Result<()> {
    let intrinsics = Intrinsics::find(module);
    log::trace!("intrinsics: {:?}", intrinsics);
    let mut mem_updates = HashMap::new();
    for directive in directives {
        log::trace!("Processing directive {:?}", directive);
        if let Some(idx) = partially_evaluate_func(module, im, &intrinsics, directive)? {
            log::trace!("New func index {}", idx);
            // Update memory image.
            mem_updates.insert(directive.func_index_out_addr, idx);
        }
    }

    // Update memory.
    let heap = im.main_heap()?;
    for (addr, value) in mem_updates {
        im.write_u32(heap, addr, value)?;
    }
    Ok(())
}

fn partially_evaluate_func(
    module: &mut Module,
    image: &Image,
    intrinsics: &Intrinsics,
    directive: &Directive,
) -> anyhow::Result<Option<u32>> {
    // Get function body.
    let body = module
        .func(directive.func)
        .body()
        .ok_or_else(|| anyhow::anyhow!("Attempt to specialize an import"))?;
    let sig = module.func(directive.func).sig();

    log::trace!("Specializing: {}", directive.func);
    log::trace!("body:\n{}", body.display("| "));

    // Compute CFG info.
    let cfg = CFGInfo::new(body);

    log::trace!("CFGInfo: {:?}", cfg);

    // Build the evaluator.
    let mut evaluator = Evaluator {
        generic: body,
        intrinsics,
        image,
        cfg,
        state: FunctionState::new(),
        func: FunctionBody::new(module, sig),
        block_map: HashMap::new(),
        block_deps: HashMap::new(),
        value_map: HashMap::new(),
        queue: VecDeque::new(),
        queue_set: HashSet::new(),
        header_blocks: HashSet::new(),
    };
    let ctx = evaluator.state.init_args(
        body,
        &mut evaluator.func,
        image,
        &directive.const_params[..],
    );
    evaluator.compute_header_blocks();
    log::trace!("after init_args, state is {:?}", evaluator.state);
    evaluator
        .queue
        .push_back((evaluator.generic.entry, ctx, evaluator.func.entry));
    evaluator.queue_set.insert((evaluator.generic.entry, ctx));
    evaluator.evaluate();

    log::debug!("Adding func:\n{}", evaluator.func.display("| "));
    let func = module.add_func(sig, evaluator.func);
    Ok(Some(func.index() as u32))
}

fn const_operator(ty: Type, value: WasmVal) -> Option<Operator> {
    match (ty, value) {
        (Type::I32, WasmVal::I32(k)) => Some(Operator::I32Const { value: k }),
        (Type::I64, WasmVal::I64(k)) => Some(Operator::I64Const { value: k }),
        (Type::F32, WasmVal::F32(k)) => Some(Operator::F32Const { value: k }),
        (Type::F64, WasmVal::F64(k)) => Some(Operator::F64Const { value: k }),
        _ => None,
    }
}

impl<'a> Evaluator<'a> {
    fn evaluate(&mut self) {
        while let Some((orig_block, ctx, new_block)) = self.queue.pop_front() {
            self.queue_set.remove(&(orig_block, ctx));
            self.evaluate_block(orig_block, ctx, new_block);
        }
    }

    fn evaluate_block(&mut self, orig_block: Block, ctx: Context, new_block: Block) {
        // Clear the block body each time we rebuild it -- we may be
        // recomputing a specialization with an existing output.
        self.func.blocks[new_block].insts.clear();

        log::trace!(
            "evaluate_block: orig {} ctx {} new {}",
            orig_block,
            ctx,
            new_block
        );

        // Create program-point state.
        let mut state = PointState {
            context: ctx,
            flow: self.state.state[ctx]
                .block_entry
                .get(&orig_block)
                .cloned()
                .unwrap(),
        };

        // Do the actual constant-prop, carrying the state across the
        // block and updating flow-sensitive state, and updating SSA
        // vals as well.
        self.evaluate_block_body(orig_block, &mut state, new_block);
        self.evaluate_term(orig_block, &mut state, new_block);
    }

    /// For a given value in the generic function, accessed in the
    /// given context and at the given block, find its abstract value
    /// and SSA value in the specialized function.
    fn use_value(
        &mut self,
        mut context: Context,
        orig_block: Block,
        orig_val: Value,
    ) -> (Value, AbstractValue) {
        log::trace!(
            "using value {} at block {} in context {}",
            orig_val,
            orig_block,
            context
        );
        let orig_context = context;
        let val_block = self.generic.value_blocks[orig_val];
        loop {
            if let Some((val, abs)) = self.state.state[context].ssa.values.get(&orig_val) {
                log::trace!(
                    " -> found specialized val {} with abstract value {:?} at context {}",
                    val,
                    abs,
                    context
                );
                self.block_deps
                    .entry((context, val_block))
                    .or_insert_with(|| HashSet::new())
                    .insert((orig_context, orig_block));
                return (*val, *abs);
            }
            assert_ne!(context, Context::default());
            context = self.state.contexts.parent(context);
            log::trace!(" -> going up to parent context {}", context);
        }
    }

    fn def_value(
        &mut self,
        block: Block,
        context: Context,
        orig_val: Value,
        val: Value,
        abs: AbstractValue,
    ) {
        log::trace!(
            "defining val {} in block {} context {} with specialized val {} abs {:?}",
            orig_val,
            block,
            context,
            val,
            abs
        );
        let changed = match self.state.state[context].ssa.values.entry(orig_val) {
            BTreeEntry::Vacant(v) => {
                v.insert((val, abs));
                true
            }
            BTreeEntry::Occupied(mut o) => {
                let val_abs = &mut o.get_mut().1;
                let updated = AbstractValue::meet(*val_abs, abs);
                let changed = updated != *val_abs;
                *val_abs = updated;
                changed
            }
        };

        if changed {
            // We need to enqueue all blocks that have read a value
            // from this block.
            if let Some(deps) = self.block_deps.remove(&(context, block)) {
                for (ctx, block) in &deps {
                    self.enqueue_block_if_existing(*block, *ctx);
                }
                self.block_deps.insert((context, block), deps);
            }
        }
    }

    fn enqueue_block_if_existing(&mut self, orig_block: Block, context: Context) {
        if let Some(block) = self.block_map.get(&(context, orig_block)).copied() {
            if self.queue_set.insert((orig_block, context)) {
                self.queue.push_back((orig_block, context, block));
            }
        }
    }

    fn evaluate_block_body(&mut self, orig_block: Block, state: &mut PointState, new_block: Block) {
        // Reused below for each instruction.
        let mut arg_abs_values = vec![];
        let mut arg_values = vec![];

        for &inst in &self.generic.blocks[orig_block].insts {
            let input_ctx = state.context;
            if let Some((result_value, result_abs)) = match &self.generic.values[inst] {
                ValueDef::Alias(_) => {
                    // Don't generate any new code; uses will be
                    // rewritten. (We resolve aliases when
                    // transcribing to specialized blocks, in other
                    // words.)
                    None
                }
                ValueDef::PickOutput(val, idx, ty) => {
                    // Directly transcribe.
                    let (val, _) = self.use_value(state.context, orig_block, *val);
                    Some((
                        ValueDef::PickOutput(val, *idx, *ty),
                        AbstractValue::Runtime(ValueTags::default()),
                    ))
                }
                ValueDef::Operator(op, args, tys) => {
                    // Collect AbstractValues for args.
                    arg_abs_values.clear();
                    arg_values.clear();
                    for &arg in args {
                        let arg = self.generic.resolve_alias(arg);
                        let (val, abs) = self.use_value(state.context, orig_block, arg);
                        arg_abs_values.push(abs);
                        arg_values.push(val);
                    }

                    // Eval the transfer-function for this operator.
                    let (result_abs_value, replace_value) = self.abstract_eval(
                        orig_block,
                        *op,
                        &arg_abs_values[..],
                        &arg_values[..],
                        state,
                    );
                    // Transcribe either the original operation, or a
                    // constant, to the output.

                    match (replace_value, result_abs_value) {
                        (_, AbstractValue::Top) => unreachable!(),
                        (Some(val), av) => Some((ValueDef::Alias(val), av)),
                        (_, AbstractValue::Concrete(bits, t)) if tys.len() == 1 => {
                            if let Some(const_op) = const_operator(tys[0], bits) {
                                Some((
                                    ValueDef::Operator(const_op, vec![], tys.clone()),
                                    AbstractValue::Concrete(bits, t),
                                ))
                            } else {
                                Some((
                                    ValueDef::Operator(
                                        *op,
                                        std::mem::take(&mut arg_values),
                                        tys.clone(),
                                    ),
                                    AbstractValue::Runtime(t),
                                ))
                            }
                        }
                        (_, av) => Some((
                            ValueDef::Operator(*op, std::mem::take(&mut arg_values), tys.clone()),
                            AbstractValue::Runtime(av.tags()),
                        )),
                    }
                }
                _ => unreachable!(
                    "Invalid ValueDef in `insts` array for {} at {}",
                    orig_block, inst
                ),
            } {
                let result_value = self.func.add_value(result_value);
                self.value_map.insert((input_ctx, inst), result_value);
                self.func.append_to_block(new_block, result_value);

                self.def_value(orig_block, input_ctx, inst, result_value, result_abs);
            }
        }
    }

    fn meet_into_block_entry(
        &mut self,
        block: Block,
        context: Context,
        state: &ProgPointState,
    ) -> bool {
        match self.state.state[context].block_entry.entry(block) {
            BTreeEntry::Vacant(v) => {
                v.insert(state.clone());
                true
            }
            BTreeEntry::Occupied(mut o) => o.get_mut().meet_with(state),
        }
    }

    fn create_block(
        &mut self,
        orig_block: Block,
        context: Context,
        state: ProgPointState,
    ) -> Block {
        let block = self.func.add_block();
        for &(ty, param) in &self.generic.blocks[orig_block].params {
            let new_param = self.func.add_blockparam(block, ty);
            self.value_map.insert((context, param), new_param);
        }
        self.block_map.insert((context, orig_block), block);
        self.state.state[context]
            .block_entry
            .insert(orig_block, state);
        block
    }

    fn target_block(
        &mut self,
        state: &PointState,
        orig_block: Block,
        target: Block,
    ) -> (Block, Context) {
        log::trace!(
            "targeting block {} from {}, in context {}",
            target,
            orig_block,
            state.context
        );

        let mut target_context = state.context;
        // Pop and/or update PC if needed.
        let updated_state = loop {
            let elem = self.state.contexts.leaf_element(target_context);
            if elem.1 == self.generic.entry {
                break Cow::Borrowed(state);
            }
            if !self.cfg.dominates(elem.1, target) {
                target_context = self.state.contexts.parent(target_context);
                log::trace!(
                    " -> header block of context {} does not dominate {}; popping to parent {}",
                    elem.1,
                    target,
                    target_context
                );
                break Cow::Borrowed(state);
            } else if elem.1 == target {
                log::trace!(
                    " -> header block of context {} is target; handling staged PC updated {:?}",
                    target_context,
                    state.flow.staged_pc
                );
                // If we have a staged PC update, make it now.
                match state.flow.staged_pc {
                    StagedPC::None => {
                        break Cow::Borrowed(state);
                    }
                    StagedPC::Conflict => {
                        let mut state = state.clone();
                        state.flow.staged_pc = StagedPC::None;
                        break Cow::Owned(state);
                    }
                    StagedPC::Some(pc) => {
                        let mut state = state.clone();
                        let parent = self.state.contexts.parent(target_context);
                        target_context = self
                            .state
                            .contexts
                            .create(Some(parent), ContextElem(pc, elem.1));
                        log::trace!(" -> new context is {} parent {}", target_context, parent);
                        state.flow.staged_pc = StagedPC::None;
                        state.context = target_context;
                        break Cow::Owned(state);
                    }
                }
            } else {
                break Cow::Borrowed(state);
            }
        };
        // Push new context elem if entering a loop.
        let updated_state =
            if self.header_blocks.contains(&target) && !self.cfg.dominates(target, orig_block) {
                let context = self
                    .state
                    .contexts
                    .create(Some(updated_state.context), ContextElem(None, target));
                let mut updated_state = updated_state.into_owned();
                updated_state.context = context;
                log::trace!(
                    "pushing context for loop header {}: now {}",
                    target,
                    context
                );
                Cow::Owned(updated_state)
            } else {
                updated_state
            };

        match self.block_map.entry((updated_state.context, target)) {
            HashEntry::Vacant(_) => {
                let block = self.create_block(target, updated_state.context, state.flow.clone());
                self.block_map
                    .insert((updated_state.context, target), block);
                self.queue_set.insert((target, updated_state.context));
                self.queue.push_back((target, updated_state.context, block));
                (block, updated_state.context)
            }
            HashEntry::Occupied(o) => {
                let target_specialized = *o.get();
                let changed =
                    self.meet_into_block_entry(target, updated_state.context, &updated_state.flow);
                if changed {
                    if self.queue_set.insert((target, updated_state.context)) {
                        self.queue
                            .push_back((target, updated_state.context, target_specialized));
                    }
                }
                (target_specialized, updated_state.context)
            }
        }
    }

    fn evaluate_block_target(
        &mut self,
        orig_block: Block,
        state: &PointState,
        target: &BlockTarget,
    ) -> BlockTarget {
        let mut args = vec![];
        let mut abs_args = vec![];
        log::trace!(
            "evaluate target: block {} context {} to {:?}",
            orig_block,
            state.context,
            target
        );

        let (target_block, target_ctx) = self.target_block(state, orig_block, target.block);

        for (blockparam, arg) in self.generic.blocks[target.block]
            .params
            .iter()
            .map(|(_, val)| *val)
            .zip(target.args.iter().copied())
        {
            let (val, abs) = self.use_value(state.context, orig_block, arg);
            args.push(val);
            abs_args.push(abs);
            log::trace!(
                "blockparam: block {} context {} to param {}: val {} abs {:?}",
                orig_block,
                state.context,
                blockparam,
                val,
                abs
            );
        }

        // Parallel-move semantics: read all uses above, then write
        // all defs below.
        for (blockparam, (val, abs)) in self.generic.blocks[target.block]
            .params
            .iter()
            .map(|(_, val)| *val)
            .zip(args.iter().zip(abs_args.iter()))
        {
            self.def_value(orig_block, target_ctx, blockparam, *val, *abs);
        }

        BlockTarget {
            block: target_block,
            args,
        }
    }

    fn evaluate_term(&mut self, orig_block: Block, state: &mut PointState, new_block: Block) {
        log::trace!(
            "evaluating terminator: block {} context {} specialized block {}: {:?}",
            orig_block,
            state.context,
            new_block,
            self.generic.blocks[orig_block].terminator
        );
        let new_term = match &self.generic.blocks[orig_block].terminator {
            &Terminator::None => Terminator::None,
            &Terminator::CondBr {
                cond,
                ref if_true,
                ref if_false,
            } => {
                let (cond, abs_cond) = self.use_value(state.context, orig_block, cond);
                match abs_cond.is_const_truthy() {
                    Some(true) => Terminator::Br {
                        target: self.evaluate_block_target(orig_block, state, if_true),
                    },
                    Some(false) => Terminator::Br {
                        target: self.evaluate_block_target(orig_block, state, if_false),
                    },
                    None => Terminator::CondBr {
                        cond,
                        if_true: self.evaluate_block_target(orig_block, state, if_true),
                        if_false: self.evaluate_block_target(orig_block, state, if_false),
                    },
                }
            }
            &Terminator::Br { ref target } => Terminator::Br {
                target: self.evaluate_block_target(orig_block, state, target),
            },
            &Terminator::Select {
                value,
                ref targets,
                ref default,
            } => {
                let (value, abs_value) = self.use_value(state.context, orig_block, value);
                if let Some(selector) = abs_value.is_const_u32() {
                    let selector = selector as usize;
                    let target = if selector < targets.len() {
                        &targets[selector]
                    } else {
                        default
                    };
                    Terminator::Br {
                        target: self.evaluate_block_target(orig_block, state, target),
                    }
                } else {
                    let targets = targets
                        .iter()
                        .map(|target| self.evaluate_block_target(orig_block, state, target))
                        .collect::<Vec<_>>();
                    let default = self.evaluate_block_target(orig_block, state, default);
                    Terminator::Select {
                        value,
                        targets,
                        default,
                    }
                }
            }
            &Terminator::Return { ref values } => {
                let values = values
                    .iter()
                    .map(|&value| self.use_value(state.context, orig_block, value).0)
                    .collect::<Vec<_>>();
                Terminator::Return { values }
            }
            &Terminator::Unreachable => Terminator::Unreachable,
        };
        self.func.set_terminator(new_block, new_term);
    }

    fn abstract_eval(
        &mut self,
        orig_block: Block,
        op: Operator,
        abs: &[AbstractValue],
        values: &[Value],
        state: &mut PointState,
    ) -> (AbstractValue, Option<Value>) {
        debug_assert_eq!(abs.len(), values.len());

        if let Some((ret, replace_val)) =
            self.abstract_eval_intrinsic(orig_block, op, abs, values, state)
        {
            return (ret, replace_val);
        }

        let ret = match abs.len() {
            0 => self.abstract_eval_nullary(op, state),
            1 => self.abstract_eval_unary(op, abs[0], values[0], state),
            2 => self.abstract_eval_binary(op, abs[0], abs[1], values[0], values[1], state),
            3 => self.abstract_eval_ternary(
                op, abs[0], abs[1], abs[2], values[0], values[1], values[2], state,
            ),
            _ => AbstractValue::Runtime(ValueTags::default()),
        };
        (ret, None)
    }

    fn abstract_eval_intrinsic(
        &mut self,
        _orig_block: Block,
        op: Operator,
        abs: &[AbstractValue],
        values: &[Value],
        state: &mut PointState,
    ) -> Option<(AbstractValue, Option<Value>)> {
        match op {
            Operator::Call { function_index } => {
                if Some(function_index) == self.intrinsics.assume_const_memory {
                    Some((abs[0].with_tags(ValueTags::const_memory()), Some(values[0])))
                } else if Some(function_index) == self.intrinsics.loop_pc32_update {
                    let pc = abs[0].is_const_u32().map(|pc| pc as u64);
                    state.flow.staged_pc = StagedPC::Some(pc);
                    log::trace!("change PC: stage {:?} for next loop backedge", pc);
                    Some((abs[0], Some(values[0])))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn abstract_eval_nullary(&mut self, op: Operator, state: &mut PointState) -> AbstractValue {
        match op {
            Operator::GlobalGet { global_index } => state
                .flow
                .globals
                .get(&global_index)
                .cloned()
                .unwrap_or(AbstractValue::Runtime(ValueTags::default())),
            Operator::I32Const { .. }
            | Operator::I64Const { .. }
            | Operator::F32Const { .. }
            | Operator::F64Const { .. } => {
                AbstractValue::Concrete(WasmVal::try_from(op).unwrap(), ValueTags::default())
            }
            _ => AbstractValue::Runtime(ValueTags::default()),
        }
    }

    fn abstract_eval_unary(
        &mut self,
        op: Operator,
        x: AbstractValue,
        _x_val: Value,
        state: &mut PointState,
    ) -> AbstractValue {
        match (op, x) {
            (Operator::GlobalSet { global_index }, av) => {
                state.flow.globals.insert(global_index, av);
                AbstractValue::Runtime(ValueTags::default())
            }
            (Operator::I32Eqz, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(if k == 0 { 1 } else { 0 }), t)
            }
            (Operator::I64Eqz, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(if k == 0 { 1 } else { 0 }), t)
            }
            (Operator::I32Extend8S, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k as i8 as i32 as u32), t)
            }
            (Operator::I32Extend16S, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k as i16 as i32 as u32), t)
            }
            (Operator::I64Extend8S, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k as i8 as i64 as u64), t)
            }
            (Operator::I64Extend16S, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k as i16 as i64 as u64), t)
            }
            (Operator::I64Extend32S, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k as i32 as i64 as u64), t)
            }
            (Operator::I32Clz, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k.leading_zeros()), t)
            }
            (Operator::I64Clz, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k.leading_zeros() as u64), t)
            }
            (Operator::I32Ctz, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k.trailing_zeros()), t)
            }
            (Operator::I64Ctz, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k.trailing_zeros() as u64), t)
            }
            (Operator::I32Popcnt, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k.count_ones()), t)
            }
            (Operator::I64Popcnt, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k.count_ones() as u64), t)
            }
            (Operator::I32WrapI64, AbstractValue::Concrete(WasmVal::I64(k), t)) => {
                AbstractValue::Concrete(WasmVal::I32(k as u32), t)
            }
            (Operator::I64ExtendI32S, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k as i32 as i64 as u64), t)
            }
            (Operator::I64ExtendI32U, AbstractValue::Concrete(WasmVal::I32(k), t)) => {
                AbstractValue::Concrete(WasmVal::I64(k as u64), t)
            }

            (Operator::I32Load { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I32Load8U { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I32Load8S { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I32Load16U { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I32Load16S { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
                if t.contains(ValueTags::const_memory()) =>
            {
                let size = match op {
                    Operator::I32Load { .. } => 4,
                    Operator::I32Load8U { .. } => 1,
                    Operator::I32Load8S { .. } => 1,
                    Operator::I32Load16U { .. } => 2,
                    Operator::I32Load16S { .. } => 2,
                    _ => unreachable!(),
                };
                let conv = |x: u64| match op {
                    Operator::I32Load { .. } => x as u32,
                    Operator::I32Load8U { .. } => x as u8 as u32,
                    Operator::I32Load8S { .. } => x as i8 as i32 as u32,
                    Operator::I32Load16U { .. } => x as u16 as u32,
                    Operator::I32Load16S { .. } => x as i16 as i32 as u32,
                    _ => unreachable!(),
                };

                self.image
                    .read_size(memory.memory, k + memory.offset as u32, size)
                    .map(|data| AbstractValue::Concrete(WasmVal::I32(conv(data)), t))
                    .unwrap_or(AbstractValue::Runtime(ValueTags::default()))
            }

            (Operator::I64Load { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load8U { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load8S { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load16U { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load16S { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load32U { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
            | (Operator::I64Load32S { memory }, AbstractValue::Concrete(WasmVal::I32(k), t))
                if t.contains(ValueTags::const_memory()) =>
            {
                let size = match op {
                    Operator::I64Load { .. } => 8,
                    Operator::I64Load8U { .. } => 1,
                    Operator::I64Load8S { .. } => 1,
                    Operator::I64Load16U { .. } => 2,
                    Operator::I64Load16S { .. } => 2,
                    Operator::I64Load32U { .. } => 4,
                    Operator::I64Load32S { .. } => 4,
                    _ => unreachable!(),
                };
                let conv = |x: u64| match op {
                    Operator::I64Load { .. } => x,
                    Operator::I64Load8U { .. } => x as u8 as u64,
                    Operator::I64Load8S { .. } => x as i8 as i64 as u64,
                    Operator::I64Load16U { .. } => x as u16 as u64,
                    Operator::I64Load16S { .. } => x as i16 as i64 as u64,
                    Operator::I64Load32U { .. } => x as u32 as u64,
                    Operator::I64Load32S { .. } => x as i32 as i64 as u64,
                    _ => unreachable!(),
                };

                self.image
                    .read_size(memory.memory, k + memory.offset as u32, size)
                    .map(|data| AbstractValue::Concrete(WasmVal::I64(conv(data)), t))
                    .unwrap_or(AbstractValue::Runtime(ValueTags::default()))
            }

            // TODO: FP and SIMD
            _ => AbstractValue::Runtime(ValueTags::default()),
        }
    }

    fn abstract_eval_binary(
        &mut self,
        op: Operator,
        x: AbstractValue,
        y: AbstractValue,
        _x_val: Value,
        _y_val: Value,
        _state: &mut PointState,
    ) -> AbstractValue {
        match (x, y) {
            (AbstractValue::Concrete(v1, tag1), AbstractValue::Concrete(v2, tag2)) => {
                let tags = tag1.meet(tag2);
                match (op, v1, v2) {
                    // 32-bit comparisons.
                    (Operator::I32Eq, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 == k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I32Ne, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 != k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I32LtS, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I32(if (k1 as i32) < (k2 as i32) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I32LtU, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 < k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I32GtS, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I32(if (k1 as i32) > (k2 as i32) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I32GtU, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 > k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I32LeS, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I32(if (k1 as i32) <= (k2 as i32) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I32LeU, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 <= k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I32GeS, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I32(if (k1 as i32) >= (k2 as i32) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I32GeU, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(if k1 >= k2 { 1 } else { 0 }), tags)
                    }

                    // 64-bit comparisons.
                    (Operator::I64Eq, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 == k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I64Ne, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 != k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I64LtS, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(if (k1 as i64) < (k2 as i64) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I64LtU, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 < k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I64GtS, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(if (k1 as i64) > (k2 as i64) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I64GtU, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 > k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I64LeS, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(if (k1 as i64) <= (k2 as i64) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I64LeU, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 <= k2 { 1 } else { 0 }), tags)
                    }
                    (Operator::I64GeS, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(if (k1 as i64) >= (k2 as i64) { 1 } else { 0 }),
                            tags,
                        )
                    }
                    (Operator::I64GeU, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(if k1 >= k2 { 1 } else { 0 }), tags)
                    }

                    // 32-bit integer arithmetic.
                    (Operator::I32Add, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_add(k2)), tags)
                    }
                    (Operator::I32Sub, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_sub(k2)), tags)
                    }
                    (Operator::I32Mul, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_mul(k2)), tags)
                    }
                    (Operator::I32DivU, WasmVal::I32(k1), WasmVal::I32(k2)) if k2 != 0 => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_div(k2)), tags)
                    }
                    (Operator::I32DivS, WasmVal::I32(k1), WasmVal::I32(k2))
                        if k2 != 0 && (k1 != 0x8000_0000 || k2 != 0xffff_ffff) =>
                    {
                        AbstractValue::Concrete(
                            WasmVal::I32((k1 as i32).wrapping_div(k2 as i32) as u32),
                            tags,
                        )
                    }
                    (Operator::I32RemU, WasmVal::I32(k1), WasmVal::I32(k2)) if k2 != 0 => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_rem(k2)), tags)
                    }
                    (Operator::I32RemS, WasmVal::I32(k1), WasmVal::I32(k2))
                        if k2 != 0 && (k1 != 0x8000_0000 || k2 != 0xffff_ffff) =>
                    {
                        AbstractValue::Concrete(
                            WasmVal::I32((k1 as i32).wrapping_rem(k2 as i32) as u32),
                            tags,
                        )
                    }
                    (Operator::I32And, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1 & k2), tags)
                    }
                    (Operator::I32Or, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1 | k2), tags)
                    }
                    (Operator::I32Xor, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1 ^ k2), tags)
                    }
                    (Operator::I32Shl, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_shl(k2 & 0x1f)), tags)
                    }
                    (Operator::I32ShrU, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(WasmVal::I32(k1.wrapping_shr(k2 & 0x1f)), tags)
                    }
                    (Operator::I32ShrS, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I32((k1 as i32).wrapping_shr(k2 & 0x1f) as u32),
                            tags,
                        )
                    }
                    (Operator::I32Rotl, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        let amt = k2 & 0x1f;
                        let result = k1.wrapping_shl(amt) | k1.wrapping_shr(32 - amt);
                        AbstractValue::Concrete(WasmVal::I32(result), tags)
                    }
                    (Operator::I32Rotr, WasmVal::I32(k1), WasmVal::I32(k2)) => {
                        let amt = k2 & 0x1f;
                        let result = k1.wrapping_shr(amt) | k1.wrapping_shl(32 - amt);
                        AbstractValue::Concrete(WasmVal::I32(result), tags)
                    }

                    // 64-bit integer arithmetic.
                    (Operator::I64Add, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1.wrapping_add(k2)), tags)
                    }
                    (Operator::I64Sub, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1.wrapping_sub(k2)), tags)
                    }
                    (Operator::I64Mul, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1.wrapping_mul(k2)), tags)
                    }
                    (Operator::I64DivU, WasmVal::I64(k1), WasmVal::I64(k2)) if k2 != 0 => {
                        AbstractValue::Concrete(WasmVal::I64(k1.wrapping_div(k2)), tags)
                    }
                    (Operator::I64DivS, WasmVal::I64(k1), WasmVal::I64(k2))
                        if k2 != 0
                            && (k1 != 0x8000_0000_0000_0000 || k2 != 0xffff_ffff_ffff_ffff) =>
                    {
                        AbstractValue::Concrete(
                            WasmVal::I64((k1 as i64).wrapping_div(k2 as i64) as u64),
                            tags,
                        )
                    }
                    (Operator::I64RemU, WasmVal::I64(k1), WasmVal::I64(k2)) if k2 != 0 => {
                        AbstractValue::Concrete(WasmVal::I64(k1.wrapping_rem(k2)), tags)
                    }
                    (Operator::I64RemS, WasmVal::I64(k1), WasmVal::I64(k2))
                        if k2 != 0
                            && (k1 != 0x8000_0000_0000_0000 || k2 != 0xffff_ffff_ffff_ffff) =>
                    {
                        AbstractValue::Concrete(
                            WasmVal::I64((k1 as i64).wrapping_rem(k2 as i64) as u64),
                            tags,
                        )
                    }
                    (Operator::I64And, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1 & k2), tags)
                    }
                    (Operator::I64Or, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1 | k2), tags)
                    }
                    (Operator::I64Xor, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(WasmVal::I64(k1 ^ k2), tags)
                    }
                    (Operator::I64Shl, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(k1.wrapping_shl((k2 & 0x3f) as u32)),
                            tags,
                        )
                    }
                    (Operator::I64ShrU, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64(k1.wrapping_shr((k2 & 0x3f) as u32)),
                            tags,
                        )
                    }
                    (Operator::I64ShrS, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        AbstractValue::Concrete(
                            WasmVal::I64((k1 as i64).wrapping_shr((k2 & 0x3f) as u32) as u64),
                            tags,
                        )
                    }
                    (Operator::I64Rotl, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        let amt = (k2 & 0x3f) as u32;
                        let result = k1.wrapping_shl(amt) | k1.wrapping_shr(64 - amt);
                        AbstractValue::Concrete(WasmVal::I64(result), tags)
                    }
                    (Operator::I64Rotr, WasmVal::I64(k1), WasmVal::I64(k2)) => {
                        let amt = (k2 & 0x3f) as u32;
                        let result = k1.wrapping_shr(amt) | k1.wrapping_shl(64 - amt);
                        AbstractValue::Concrete(WasmVal::I64(result), tags)
                    }

                    // TODO: FP and SIMD ops.
                    _ => AbstractValue::Runtime(ValueTags::default()),
                }
            }
            _ => AbstractValue::Runtime(ValueTags::default()),
        }
    }

    fn abstract_eval_ternary(
        &mut self,
        op: Operator,
        x: AbstractValue,
        y: AbstractValue,
        z: AbstractValue,
        _x_val: Value,
        _y_val: Value,
        _z_val: Value,
        _state: &mut PointState,
    ) -> AbstractValue {
        match (op, x) {
            (Operator::Select, AbstractValue::Concrete(v, _t))
            | (Operator::TypedSelect { .. }, AbstractValue::Concrete(v, _t)) => {
                if v.is_truthy() {
                    y
                } else {
                    z
                }
            }
            _ => AbstractValue::Runtime(ValueTags::default()),
        }
    }

    fn compute_header_blocks(&mut self) {
        for (block, block_def) in self.generic.blocks.entries() {
            for &inst in &block_def.insts {
                if let ValueDef::Operator(Operator::Call { function_index }, ..) =
                    &self.generic.values[inst]
                {
                    if Some(*function_index) == self.intrinsics.loop_header {
                        self.header_blocks.insert(block);
                        break;
                    }
                }
            }
        }
    }
}
