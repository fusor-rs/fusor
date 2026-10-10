//! Portable routing through the same transition engine used by the DOM adapter.
use fusor::{Owner, OwnerHandle, render::Scope as _};
use fusor_router::{
    AppUrl,
    view::{Navigation, RouteScope, RouteView, ViewRouter},
};
use std::{any::Any, cell::RefCell, rc::Rc};

type Target = Rc<RefCell<Vec<Rc<String>>>>;

struct Scope {
    owner: Owner,
    node: Rc<String>,
    attached: Option<Target>,
    kept: Vec<Rc<dyn Any>>,
    fail: bool,
}

impl Scope {
    fn new(parent: &OwnerHandle, label: String) -> Self {
        Self {
            owner: Owner::child(parent),
            node: Rc::new(label),
            attached: None,
            kept: Vec::new(),
            fail: false,
        }
    }
}
impl fusor::render::Scope for Scope {
    fn owner(&self) -> OwnerHandle {
        self.owner.handle()
    }
    fn retain_state<T: 'static>(&mut self, value: T) -> Rc<T> {
        let value = Rc::new(value);
        self.kept.push(value.clone());
        value
    }
}
impl RouteScope for Scope {
    type Target = Target;
    type Error = String;
    fn error(message: &str) -> String {
        message.to_owned()
    }
    fn prepare_at(&mut self, target: &Target, _: bool) -> Result<(), String> {
        assert!(!self.owner().is_active());
        target.borrow_mut().push(self.node.clone());
        self.attached = Some(target.clone());
        if self.fail {
            Err("attachment failed".into())
        } else {
            Ok(())
        }
    }
    fn commit(&self) {
        self.owner.commit();
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.owner.dispose();
        if let Some(target) = &self.attached {
            target
                .borrow_mut()
                .retain(|node| !Rc::ptr_eq(node, &self.node));
        }
    }
}

fn url(path: &str) -> AppUrl {
    AppUrl::parse(path).unwrap()
}
fn only(target: &Target) -> Rc<String> {
    let target = target.borrow();
    assert_eq!(target.len(), 1);
    target[0].clone()
}

fn member_route(
    index: usize,
    prepared: Rc<RefCell<Vec<AppUrl>>>,
) -> Result<RouteView<Scope>, String> {
    RouteView::new("members/:member", move |parent, matched| {
        let destination = Navigation::<Scope>::from_owner(parent)
            .unwrap()
            .location()
            .get();
        prepared.borrow_mut().push(destination);
        let mut scope = Scope::new(parent, matched.params["member"].clone());
        scope.fail = index == 1 && matched.params["member"] == "bad";
        Ok(scope)
    })
}

fn team_route(
    children: [Target; 2],
    prepared: Rc<RefCell<Vec<AppUrl>>>,
) -> Result<RouteView<Scope>, String> {
    RouteView::new("/teams/:team/*", move |parent, matched| {
        let mut scope = Scope::new(parent, matched.params["team"].clone());
        for (index, target) in children.iter().enumerate() {
            let route = member_route(index, prepared.clone())?;
            let nested = ViewRouter::mount(&scope.owner(), target, vec![route], url("/ignored"))?;
            assert_eq!(
                nested.navigation().location().get(),
                Navigation::<Scope>::from_owner(&scope.owner())
                    .unwrap()
                    .location()
                    .get()
            );
            scope.retain_state(nested);
        }
        Ok(scope)
    })
}

#[test]
fn nested_transitions_retain_identity_and_roll_back_all_siblings_on_failure() {
    let owner = Owner::new();
    let root = Target::default();
    let children = [Target::default(), Target::default()];
    let prepared = Rc::new(RefCell::new(Vec::new()));
    let route = team_route(children.clone(), prepared.clone()).unwrap();
    let router = ViewRouter::mount(
        &owner.handle(),
        &root,
        vec![route],
        url("/teams/%CE%B1/members/1"),
    )
    .unwrap();
    let navigation = router.navigation();
    owner.commit();
    let team = only(&root);
    assert_eq!(&*team, "α");
    let member = only(&children[0]);
    navigation
        .navigate(url("/teams/%CE%B1/members/1?q=2#details"))
        .unwrap();
    assert!(Rc::ptr_eq(&team, &only(&root)));
    assert!(Rc::ptr_eq(&member, &only(&children[0])));

    navigation.navigate(url("/teams/%CE%B1/members/2")).unwrap();
    assert!(Rc::ptr_eq(&team, &only(&root)));
    assert!(!Rc::ptr_eq(&member, &only(&children[0])));
    let retained = children.each_ref().map(only);
    assert_eq!(
        navigation.navigate(url("/teams/%CE%B1/members/bad")),
        Err("attachment failed".into())
    );
    for (target, retained) in children.iter().zip(&retained) {
        assert!(
            Rc::ptr_eq(&only(target), retained),
            "earlier staged siblings also roll back"
        );
    }
    assert_eq!(navigation.location().get(), url("/teams/%CE%B1/members/2"));
    assert_eq!(
        prepared.borrow().last(),
        Some(&url("/teams/%CE%B1/members/bad"))
    );

    navigation.navigate(url("/teams/b/members/3")).unwrap();
    assert!(!Rc::ptr_eq(&team, &only(&root)));
    assert_eq!(&*only(&children[0]), "3");
    owner.dispose();
    assert!(root.borrow().is_empty());
    assert!(children.iter().all(|target| target.borrow().is_empty()));
    assert!(navigation.navigate(url("/teams/b/members/4")).is_err());
}

#[test]
fn abandoned_navigation_and_mount_clones_keep_the_current_view_alive() {
    let owner = Owner::new();
    owner.commit();
    let target = Target::default();
    let route = RouteView::new("/:id", |parent, matched| {
        Ok(Scope::new(parent, matched.params["id"].clone()))
    })
    .unwrap();
    let mounted = ViewRouter::mount(&owner.handle(), &target, vec![route], url("/one")).unwrap();
    let clone = mounted.clone();
    let navigation = mounted.navigation();
    drop(mounted);
    let original = only(&target);
    let stage = navigation.prepare_navigation(&url("/two")).unwrap();
    assert_eq!(
        target.borrow().len(),
        2,
        "candidate attachment precedes publication"
    );
    assert_eq!(navigation.location().get(), url("/one"));
    assert!(navigation.prepare_navigation(&url("/three")).is_err());
    drop(stage);
    assert!(Rc::ptr_eq(&original, &only(&target)));
    navigation.navigate(url("/two")).unwrap();
    drop(clone);
    assert!(target.borrow().is_empty());
    assert!(navigation.navigate(url("/three")).is_err());
}

#[test]
fn invalid_view_ownership_is_rejected_before_attachment() {
    let owner = Owner::new();
    let unrelated = Owner::new();
    let target = Target::default();
    let route = RouteView::new("/", move |_, _| {
        Ok(Scope::new(&unrelated.handle(), "invalid".into()))
    })
    .unwrap();
    let failure = ViewRouter::mount(&owner.handle(), &target, vec![route], url("/"));
    assert!(failure.err().unwrap().contains("prepared child"));
    assert!(target.borrow().is_empty());
}

#[test]
fn surviving_nested_mount_handles_do_not_retain_disposed_views() {
    let owner = Owner::new();
    let target = Target::default();
    let nested_target = Target::default();
    let retained = Rc::new(RefCell::new(None));
    let route = RouteView::new("/*", {
        let (retained, target) = (retained.clone(), nested_target.clone());
        move |parent, _| {
            let scope = Scope::new(parent, "parent".into());
            let child = RouteView::new("/", |parent, _| Ok(Scope::new(parent, "child".into())))?;
            *retained.borrow_mut() = Some(ViewRouter::mount(
                &scope.owner(),
                &target,
                vec![child],
                url("/"),
            )?);
            Ok(scope)
        }
    })
    .unwrap();
    let router = ViewRouter::mount(&owner.handle(), &target, vec![route], url("/")).unwrap();
    owner.commit();
    assert_eq!(&*only(&nested_target), "child");
    drop(router);
    assert!(nested_target.borrow().is_empty());
    assert!(
        retained
            .borrow()
            .as_ref()
            .unwrap()
            .navigation()
            .navigate(url("/"))
            .is_err()
    );
}

#[test]
fn staged_navigation_cannot_republish_a_disposed_nested_outlet() {
    let owner = Owner::new();
    let target = Target::default();
    let nested_target = Target::default();
    let nested_owner = Rc::new(RefCell::new(None));
    let route = RouteView::new("/team/*", {
        let (nested_owner, target) = (nested_owner.clone(), nested_target.clone());
        move |parent, _| {
            let mut scope = Scope::new(parent, "team".into());
            let child_owner = Rc::new(Owner::child(&scope.owner()));
            let child = RouteView::new("/:id", |parent, matched| {
                Ok(Scope::new(parent, matched.params["id"].clone()))
            })?;
            scope.retain_state(ViewRouter::mount(
                &child_owner.handle(),
                &target,
                vec![child],
                url("/"),
            )?);
            child_owner.commit();
            *nested_owner.borrow_mut() = Some(child_owner);
            Ok(scope)
        }
    })
    .unwrap();
    let router =
        ViewRouter::mount(&owner.handle(), &target, vec![route], url("/team/one")).unwrap();
    owner.commit();
    let stage = router
        .navigation()
        .prepare_navigation(&url("/team/two"))
        .unwrap();
    nested_owner.borrow().as_ref().unwrap().dispose();
    stage.commit();
    assert!(
        nested_target.borrow().is_empty(),
        "a prepared view must not survive its outlet owner"
    );
}
