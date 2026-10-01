#[derive(Clone, Copy, PartialEq)]
pub struct CodeToken {
    pub id: usize,
    pub text: &'static str,
    pub style: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
pub struct CodeData {
    pub label: &'static str,
    pub tokens: &'static [CodeToken],
}

pub struct CodeBlock {
    data: CodeData,
}

impl CodeBlock {
    pub fn new(data: CodeData) -> Self {
        Self { data }
    }
}

/// A declaration without the code block's label bar, for reference entries.
#[derive(fusor::FromInputs)]
pub struct Signature {
    #[input]
    tokens: &'static [CodeToken],
}

struct Token {
    item: fusor::Memo<CodeToken>,
}

fusor::bindings!(code);

impl fusor::dom::FromInputs for Token {
    type Inputs = Self;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

pub struct CodeBlockInputs {
    pub data: CodeData,
}
impl fusor::dom::FromInputs for CodeBlock {
    type Inputs = CodeBlockInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.data))
    }
}
