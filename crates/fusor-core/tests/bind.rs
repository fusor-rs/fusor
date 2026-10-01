use fusor::{
    bind::{Checkbox, TextValue, selected},
    effect, signal,
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn text_adapter_preserves_invalid_drafts_and_equivalent_edits_until_model_changes() {
    let model = signal(12_i32);
    let draft = Rc::new(RefCell::new("12".to_owned()));
    let _binding = effect({
        let (model, draft) = (model.clone(), draft.clone());
        move || {
            if !model.shows(&draft.borrow()) {
                *draft.borrow_mut() = model.text();
            }
        }
    });
    *draft.borrow_mut() = "-".into();
    model.edit(draft.borrow().clone());
    assert_eq!(model.get(), 12);
    assert_eq!(&*draft.borrow(), "-");
    *draft.borrow_mut() = "012".into();
    model.edit(draft.borrow().clone());
    assert_eq!(model.get(), 12);
    assert_eq!(&*draft.borrow(), "012");
    model.set(13);
    assert_eq!(&*draft.borrow(), "13");
}

#[test]
fn checkbox_adapters_reuse_boolean_and_parsed_membership_behavior() {
    let flag = signal(false);
    flag.check("unused", true);
    assert!(flag.checked("unused"));
    flag.check("unused", false);
    assert!(!flag.get());

    let choices = signal(vec![1_u32]);
    choices.check("2", true);
    choices.check("02", true);
    choices.check("invalid", true);
    assert_eq!(choices.get(), [1, 2]);
    assert!(selected(&choices, "02"));
    choices.check("2", false);
    assert_eq!(choices.get(), [1]);
    assert!(!choices.checked("2"));
}
