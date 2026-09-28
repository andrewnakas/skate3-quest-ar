//! APT's compact ActionScript instruction stream. All execution is bounded;
//! native callbacks are supplied by the HUD owner, never by extracted scripts.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Undefined,
    Number(f64),
    Bool(bool),
    Text(String),
    Object(usize),
}
impl Value {
    pub fn number(&self) -> f64 {
        match self {
            Self::Number(n) => *n,
            Self::Bool(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Text(s) => s.parse().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
    pub fn truth(&self) -> bool {
        match self {
            Self::Undefined => false,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::Bool(v) => *v,
            Self::Text(s) => !s.is_empty(),
            Self::Object(_) => true,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Undefined => "undefined".into(),
            Self::Number(n) => n.to_string(),
            Self::Bool(v) => v.to_string(),
            Self::Text(s) => s.clone(),
            Self::Object(_) => "[object Object]".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Constant {
    pub kind: u32,
    pub value: serde_json::Value,
}
impl Constant {
    fn value(&self) -> Result<Value, String> {
        Ok(match self.kind {
            1 => Value::Text(
                self.value
                    .as_str()
                    .ok_or("Invalid APT string constant")?
                    .into(),
            ),
            5 => Value::Bool(self.value.as_u64().ok_or("Invalid APT bool constant")? != 0),
            6 | 7 => Value::Number(self.value.as_f64().ok_or("Invalid APT number constant")?),
            3 => Value::Undefined,
            _ => return Err(format!("Unresolved APT constant type {}", self.kind)),
        })
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Parameter {
    pub register: usize,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Instruction {
    pub offset: u32,
    pub opcode: u8,
    #[serde(default)]
    pub next: u32,
    #[serde(default)]
    pub operand: serde_json::Value,
    pub target: Option<u32>,
    #[serde(default)]
    pub values: Vec<Constant>,
    #[serde(default)]
    pub body: Vec<Instruction>,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum ObjectKind {
    Plain,
    Function(usize),
    Native(String),
}
#[derive(Clone, Debug)]
pub struct Object {
    pub kind: ObjectKind,
    pub fields: BTreeMap<String, Value>,
    pub prototype: Option<usize>,
}
#[derive(Clone, Debug)]
struct Function {
    code: Instruction,
    constants: Vec<Value>,
}

struct Scope {
    this: usize,
    local_definitions: bool,
    locals: BTreeMap<String, Value>,
}

pub trait Host {
    fn property_changed(&mut self, _vm: &mut Vm, _object: usize, _key: &str) -> Result<(), String> {
        Ok(())
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        object: usize,
        method: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, String>;
}
#[derive(Default)]
pub struct Vm {
    pub objects: Vec<Object>,
    free_objects: Vec<usize>,
    functions: Vec<Function>,
    pub global: usize,
    remaining: usize,
    depth: usize,
}
impl Vm {
    fn variable(&self, scope: &Scope, key: &str) -> Value {
        if let Some(value) = scope.locals.get(key) {
            return value.clone();
        }
        let value = self.get(scope.this, key);
        if value != Value::Undefined {
            return value;
        }
        self.get(self.global, key)
    }
    pub fn new() -> Self {
        let mut vm = Self::default();
        vm.global = vm.object(ObjectKind::Plain);
        vm
    }
    pub fn object(&mut self, kind: ObjectKind) -> usize {
        if let Some(id) = self.free_objects.pop() {
            self.objects[id] = Object {
                kind,
                fields: BTreeMap::new(),
                prototype: None,
            };
            return id;
        }
        let id = self.objects.len();
        self.objects.push(Object {
            kind,
            fields: BTreeMap::new(),
            prototype: None,
        });
        id
    }
    /// Collect only between updates. Host-owned movie handles are explicit
    /// roots; script references and prototypes preserve retired clips when
    /// they remain reachable. Stable handles never move during collection.
    pub fn collect(&mut self, host_roots: impl IntoIterator<Item = usize>) -> Result<(), String> {
        if self.depth != 0 {
            return Err("Cannot collect during an APT call".into());
        }
        let mut marked = vec![false; self.objects.len()];
        let mut pending: Vec<_> = host_roots.into_iter().collect();
        pending.push(self.global);
        while let Some(id) = pending.pop() {
            let seen = marked.get_mut(id).ok_or("Invalid APT collection root")?;
            if *seen {
                continue;
            }
            *seen = true;
            let object = &self.objects[id];
            if let Some(id) = object.prototype {
                pending.push(id);
            }
            for value in object.fields.values() {
                if let Value::Object(id) = value {
                    pending.push(*id);
                }
            }
        }
        self.free_objects.clear();
        for (id, live) in marked.into_iter().enumerate() {
            if !live {
                self.objects[id] = Object {
                    kind: ObjectKind::Plain,
                    fields: BTreeMap::new(),
                    prototype: None,
                };
                self.free_objects.push(id);
            }
        }
        Ok(())
    }
    pub fn set(
        &mut self,
        object: usize,
        key: impl Into<String>,
        value: Value,
    ) -> Result<(), String> {
        self.objects
            .get_mut(object)
            .ok_or("Invalid APT object handle")?
            .fields
            .insert(key.into(), value);
        Ok(())
    }
    pub fn get(&self, object: usize, key: &str) -> Value {
        let mut current = Some(object);
        for _ in 0..64 {
            let Some(o) = current.and_then(|i| self.objects.get(i)) else {
                break;
            };
            if let Some(v) = o.fields.get(key) {
                return v.clone();
            }
            current = o.prototype;
        }
        Value::Undefined
    }
    pub fn begin_update(&mut self) {
        self.remaining = 100_000;
    }
    pub fn construct(
        &mut self,
        class: usize,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<usize, String> {
        let id = self.object(ObjectKind::Plain);
        if let Value::Object(proto) = self.get(class, "prototype") {
            self.objects[id].prototype = Some(proto);
        }
        if let ObjectKind::Function(f) = self.objects[class].kind.clone() {
            self.invoke(f, id, args, host)?;
        }
        Ok(id)
    }
    pub fn run(&mut self, code: &[Instruction], host: &mut impl Host) -> Result<Value, String> {
        self.begin_update();
        self.run_on(self.global, code, host)
    }
    pub fn run_on(
        &mut self,
        object: usize,
        code: &[Instruction],
        host: &mut impl Host,
    ) -> Result<Value, String> {
        self.execute(
            code,
            &mut vec![Value::Undefined; 256],
            &mut Vec::new(),
            &mut Scope {
                this: object,
                local_definitions: false,
                locals: BTreeMap::new(),
            },
            host,
        )
    }
    pub fn call_method(
        &mut self,
        object: usize,
        method: &str,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        if let Value::Object(f) = self.get(object, method) {
            if let Some(Object {
                kind: ObjectKind::Function(index),
                ..
            }) = self.objects.get(f)
            {
                return self.invoke(*index, object, args, host);
            }
        }
        host.call(self, object, method, args)
    }
    fn invoke(
        &mut self,
        index: usize,
        this: usize,
        args: Vec<Value>,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        if self.depth >= 32 {
            return Err("APT call-depth limit".into());
        }
        let f = self
            .functions
            .get(index)
            .ok_or("Invalid APT function")?
            .clone();
        let mut regs = vec![Value::Undefined; 256];
        let mut reg = 1;
        let mut scope = Scope {
            this,
            local_definitions: true,
            locals: BTreeMap::new(),
        };
        if f.code.flags & 2 == 0 {
            scope.locals.insert("this".into(), Value::Object(this));
        }
        let root = match self.get(this, "_root") {
            Value::Undefined => Value::Object(self.global),
            value => value,
        };
        let parent = self.get(this, "_parent");
        let arguments = if f.code.flags & 4 != 0 || f.code.flags & 8 == 0 {
            let object = self.object(ObjectKind::Plain);
            for (index, value) in args.iter().enumerate() {
                self.set(object, index.to_string(), value.clone())?;
            }
            self.set(object, "length", Value::Number(args.len() as f64))?;
            Value::Object(object)
        } else {
            Value::Undefined
        };
        if f.code.flags & 8 == 0 {
            scope.locals.insert("arguments".into(), arguments.clone());
        }
        // DefineFunction2 preload flags (82E47688). Native integer registers
        // are scoped to a call, unlike persistent display-object properties.
        for (flag, value) in [
            (1, Value::Object(this)),
            (4, arguments),
            (16, Value::Undefined),
            (64, root),
            (128, parent),
            (256, Value::Object(self.global)),
        ] {
            if f.code.flags & flag != 0 {
                regs[reg] = value;
                reg += 1;
            }
        }
        for (i, p) in f.code.parameters.iter().enumerate() {
            if p.register >= regs.len() {
                return Err("APT parameter register outside bank".into());
            }
            if p.register != 0 {
                regs[p.register] = args.get(i).cloned().unwrap_or_default();
            } else {
                scope
                    .locals
                    .insert(p.name.clone(), args.get(i).cloned().unwrap_or_default());
            }
        }
        self.depth += 1;
        let result = self.execute(
            &f.code.body,
            &mut regs,
            &mut f.constants.clone(),
            &mut scope,
            host,
        );
        self.depth -= 1;
        result
    }
    fn execute(
        &mut self,
        code: &[Instruction],
        regs: &mut Vec<Value>,
        constants: &mut Vec<Value>,
        scope: &mut Scope,
        host: &mut impl Host,
    ) -> Result<Value, String> {
        let mut stack = Vec::<Value>::new();
        let mut pc = 0;
        fn pop(s: &mut Vec<Value>) -> Result<Value, String> {
            s.pop().ok_or("APT stack underflow".into())
        }
        while let Some(i) = code.get(pc) {
            if self.remaining == 0 || self.objects.len() > 4096 {
                return Err("APT execution/object budget exceeded".into());
            }
            self.remaining -= 1;
            pc += 1;
            let op = i.opcode;
            let operand = i.operand.as_u64().unwrap_or(0) as usize;
            let constant = |k: usize| {
                constants
                    .get(k)
                    .cloned()
                    .ok_or_else(|| format!("APT constant index {k} at {:x}", i.offset))
            };
            match op {
                0 => break,
                0x70 => stack.push(Value::Object(scope.this)),
                0xa1 => stack.push(Value::Text(
                    i.operand
                        .as_str()
                        .ok_or("APT string operand missing")?
                        .into(),
                )),
                0xa4 => stack.push(self.variable(
                    scope,
                    i.operand.as_str().ok_or("APT variable operand missing")?,
                )),
                0x06 | 0x07 => {
                    host.call(
                        self,
                        scope.this,
                        if op == 6 { "play" } else { "stop" },
                        vec![],
                    )?;
                }
                0x17 => {
                    pop(&mut stack)?;
                }
                0x12 => {
                    let a = pop(&mut stack)?;
                    stack.push(Value::Bool(!a.truth()));
                }
                0x4c => stack.push(
                    stack
                        .last()
                        .cloned()
                        .ok_or("APT duplicate on empty stack")?,
                ),
                0x59 => stack.push(Value::Number(0.0)),
                0x5a => stack.push(Value::Number(1.0)),
                0x71 => stack.push(Value::Object(self.global)),
                0x73 => stack.push(Value::Bool(true)),
                0x74 => stack.push(Value::Bool(false)),
                0x75 | 0x76 => stack.push(Value::Undefined),
                0xb5 => stack.push(Value::Number((operand as u8 as i8) as f64)),
                0xb6 => stack.push(Value::Number((operand as u16 as i16) as f64)),
                0xb7 => stack.push(Value::Number((operand as u32 as i32) as f64)),
                0xb4 => stack.push(Value::Number(f32::from_bits(operand as u32) as f64)),
                0xb9 => stack.push(
                    regs.get(operand)
                        .cloned()
                        .ok_or("APT register outside bank")?,
                ),
                0x87 => {
                    *regs.get_mut(operand).ok_or("APT register outside bank")? = stack
                        .last()
                        .cloned()
                        .ok_or("APT register store on empty stack")?;
                }
                0x88 => {
                    *constants = i
                        .values
                        .iter()
                        .map(Constant::value)
                        .collect::<Result<_, _>>()?;
                }
                0x96 => {
                    for c in &i.values {
                        stack.push(c.value()?);
                    }
                }
                0xa2 | 0xa3 => stack.push(constant(operand)?),
                0xae => stack.push(self.variable(scope, &constant(operand)?.text())),
                0x4e | 0xaf => {
                    let name = if op == 0xaf {
                        constant(operand)?
                    } else {
                        pop(&mut stack)?
                    }
                    .text();
                    let obj = pop(&mut stack)?;
                    stack.push(if let Value::Object(id) = obj {
                        self.get(id, &name)
                    } else {
                        Value::Undefined
                    });
                }
                0x4f => {
                    let v = pop(&mut stack)?;
                    let k = pop(&mut stack)?.text();
                    let o = pop(&mut stack)?;
                    if let Value::Object(id) = o {
                        self.set(id, &k, v)?;
                        host.property_changed(self, id, &k)?;
                    }
                }
                0x1c => {
                    let k = pop(&mut stack)?.text();
                    stack.push(self.variable(scope, &k));
                }
                0x1d | 0x3c => {
                    let v = pop(&mut stack)?;
                    let k = pop(&mut stack)?.text();
                    if (op == 0x3c && scope.local_definitions) || scope.locals.contains_key(&k) {
                        scope.locals.insert(k, v);
                    } else {
                        self.set(scope.this, &k, v)?;
                        host.property_changed(self, scope.this, &k)?;
                    }
                }
                0x3a => {
                    let k = pop(&mut stack)?.text();
                    let o = pop(&mut stack)?;
                    let existed = if let Value::Object(id) = o {
                        self.objects[id].fields.remove(&k).is_some()
                    } else {
                        false
                    };
                    stack.push(Value::Bool(existed));
                }
                0x47 | 0x0b | 0x0c | 0x0d | 0x48 | 0x49 | 0x66 | 0x67 => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(match op {
                        0x47 if matches!(a, Value::Text(_)) || matches!(b, Value::Text(_)) => {
                            Value::Text(a.text() + &b.text())
                        }
                        0x47 => Value::Number(a.number() + b.number()),
                        0x0b => Value::Number(a.number() - b.number()),
                        0x0c => Value::Number(a.number() * b.number()),
                        0x0d => Value::Number(a.number() / b.number()),
                        0x48 => Value::Bool(a.number() < b.number()),
                        0x67 => Value::Bool(a.number() > b.number()),
                        0x66 => Value::Bool(a == b),
                        _ => Value::Bool(a == b || a.number() == b.number()),
                    });
                }
                0x50 | 0x51 => {
                    let a = pop(&mut stack)?.number();
                    stack.push(Value::Number(a + if op == 0x50 { 1.0 } else { -1.0 }));
                }
                0x99 | 0x9d | 0xb8 => {
                    let jump = op == 0x99 || {
                        let v = pop(&mut stack)?.truth();
                        if op == 0xb8 { !v } else { v }
                    };
                    if jump {
                        let target = i.target.ok_or("APT branch lacks target")?;
                        if target == code.last().map_or(0, |x| x.next) {
                            break;
                        }
                        pc = code
                            .iter()
                            .position(|x| x.offset == target)
                            .ok_or_else(|| format!("APT invalid jump target {target:x}"))?;
                    }
                }
                0x8e | 0x9b => {
                    let index = self.functions.len();
                    self.functions.push(Function {
                        code: i.clone(),
                        constants: constants.clone(),
                    });
                    let object = self.object(ObjectKind::Function(index));
                    let proto = self.object(ObjectKind::Plain);
                    self.set(object, "prototype", Value::Object(proto))?;
                    if i.name.is_empty() {
                        stack.push(Value::Object(object));
                    } else {
                        self.set(self.global, &i.name, Value::Object(object))?;
                    }
                }
                0x69 => {
                    let super_class = pop(&mut stack)?;
                    let sub_class = pop(&mut stack)?;
                    if let (Value::Object(a), Value::Object(b)) = (sub_class, super_class) {
                        if let (Value::Object(ap), Value::Object(bp)) =
                            (self.get(a, "prototype"), self.get(b, "prototype"))
                        {
                            self.objects[ap].prototype = Some(bp);
                        }
                    }
                }
                0x40 => {
                    let name = pop(&mut stack)?.text();
                    let n = pop(&mut stack)?.number() as usize;
                    if n > 256 {
                        return Err("APT argument limit".into());
                    }
                    let args = (0..n)
                        .map(|_| pop(&mut stack))
                        .collect::<Result<Vec<_>, _>>()?;
                    let id = self.object(ObjectKind::Plain);
                    if name == "Array" {
                        for (j, v) in args.into_iter().enumerate() {
                            self.set(id, j.to_string(), v)?;
                        }
                        self.set(id, "length", Value::Number(n as f64))?;
                    } else if let Value::Object(class) = self.get(self.global, &name) {
                        if let Value::Object(proto) = self.get(class, "prototype") {
                            self.objects[id].prototype = Some(proto);
                        }
                        if let ObjectKind::Function(f) = self.objects[class].kind.clone() {
                            self.invoke(f, id, args, host)?;
                        }
                    } else {
                        return Err(format!("APT constructor absent: {name}"));
                    }
                    stack.push(Value::Object(id));
                }
                0x52 | 0xb0 | 0xb1 | 0xb2 | 0xb3 | 0x3d | 0x5d => {
                    let method = if matches!(op, 0xb0 | 0xb1 | 0xb2 | 0xb3) {
                        constant(operand)?
                    } else {
                        pop(&mut stack)?
                    };
                    let object = if matches!(op, 0x3d | 0xb0 | 0xb1) {
                        Value::Object(self.global)
                    } else {
                        pop(&mut stack)?
                    };
                    let count = pop(&mut stack)?.number() as usize;
                    if count > 256 {
                        return Err("APT argument limit".into());
                    }
                    let args = (0..count)
                        .map(|_| pop(&mut stack))
                        .collect::<Result<Vec<_>, _>>()?;
                    let value = if let Value::Object(id) = object {
                        self.call_method(id, &method.text(), args, host)?
                    } else {
                        Value::Undefined
                    };
                    if matches!(op, 0xb1 | 0xb3) {
                        return Ok(value);
                    }
                    if !matches!(op, 0xb0 | 0xb2 | 0x5d) {
                        stack.push(value);
                    }
                }
                0x3e => return Ok(stack.pop().unwrap_or_default()),
                _ => return Err(format!("Unsupported APT opcode {op:02x} at {:x}", i.offset)),
            }
            if stack.len() > 1024 {
                return Err("APT stack limit".into());
            }
        }
        Ok(Value::Undefined)
    }
}
