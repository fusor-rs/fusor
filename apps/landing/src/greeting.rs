use fusor::prelude::*;

#[derive(FromInputs)]
pub struct Greeting {
    #[local(init = signal(String::new()))]
    name: Signal<String>,
}

impl Greeting {
    fn line(&self) -> String {
        let name = self.name.get();
        match name.trim() {
            "" => "Hello.".to_owned(),
            name => format!("Hello, {name}."),
        }
    }
}

fusor::template!("web/components/greeting.html");
