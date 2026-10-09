use super::*;

fn slot_name(source: &str, value: &Spanned<HtmlString, usize>) -> Result<String, ExtractError> {
    let name = String::from_utf8_lossy(value);
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(error(
            source,
            crate::html::value_start(source, value.span.start),
            "slot names must be nonempty literal names containing ASCII letters, digits, _ or -",
        ));
    }
    Ok(name.into_owned())
}

pub(super) fn placement_name(
    source: &str,
    tag: &StartTag<usize>,
) -> Result<Option<String>, ExtractError> {
    if tag.self_closing || tag.attributes.keys().any(|key| key.as_ref() != b"name") {
        return Err(error(
            source,
            tag.span.start,
            "Children accepts only an optional name and requires an explicit closing tag",
        ));
    }
    tag.attributes
        .get(b"name".as_slice())
        .map(|value| slot_name(source, value))
        .transpose()
}

impl Parser<'_> {
    fn named_children_owner(
        &self,
        tag: &StartTag<usize>,
        cx: &TagContext,
    ) -> Result<(&InvocationFrame, String), ExtractError> {
        let Some(Frame {
            kind: FrameKind::Invocation(invocation),
            ..
        }) = cx.parent(&self.stack)
        else {
            return Err(error(
                self.source,
                tag.span.start,
                "a named slot template must be a direct child of a component tag",
            ));
        };
        if tag.self_closing || tag.attributes.len() != 1 {
            return Err(error(
                self.source,
                tag.span.start,
                "a named slot template accepts only slot and requires an explicit closing tag",
            ));
        }
        let name = slot_name(self.source, &tag.attributes[b"slot".as_slice()])?;
        let Binding::Invocation { children, .. } =
            &self.components[invocation.caller].bindings[invocation.binding]
        else {
            unreachable!("named slots belong to a component invocation")
        };
        if children
            .iter()
            .any(|child| child.name.as_ref() == Some(&name))
        {
            return Err(error(
                self.source,
                tag.span.start,
                format!("slot {name:?} is supplied more than once"),
            ));
        }
        Ok((invocation, name))
    }

    pub(super) fn open_children_fragment(
        &mut self,
        tag: &StartTag<usize>,
        cx: &TagContext,
        owner: usize,
    ) -> usize {
        let index = self.components.len();
        let id = ComponentId::new(self.first_component + index);
        let owner = &self.components[owner];
        self.components.push(Component {
            row_locals: owner.row_locals.clone(),
            ..cx.lexicals.open(Component::new(
                id,
                Rust::ident(&format!("__FusorChildren{}", id.index()), tag.span.start),
                ComponentShape::Fragment(owner.ty.clone()),
                owner.render,
                tag.span.end..tag.span.end,
            ))
        });
        index
    }

    pub(super) fn start_named_children(
        &mut self,
        tag: StartTag<usize>,
        cx: TagContext,
    ) -> Result<(), ExtractError> {
        let (invocation, name) = self.named_children_owner(&tag, &cx)?;
        let (caller, binding) = (invocation.caller, invocation.binding);
        let index = self.open_children_fragment(&tag, &cx, caller);
        let Binding::Invocation { children, .. } = &mut self.components[caller].bindings[binding]
        else {
            unreachable!("named slots belong to a component invocation")
        };
        children.push(ChildFragment {
            name: Some(name),
            body: index,
        });
        self.edits.push(replace_tag(&tag.span, ""));
        self.stack.push(Frame::owned(
            cx.name,
            index,
            ElementId::new(self.node),
            FrameKind::NamedChildren,
        ));
        Ok(())
    }
}
