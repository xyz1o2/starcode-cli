
pub struct User {
    pub name: String,
    pub age: u32,
}

impl User {
    pub fn new(name: String, age: u32) -> Self {
        Self { name, age }
    }

    pub fn greet(&self) -> String {
        format!("Hello, my name is {}", self.name)
    }
}

pub fn main() {
    let user = User::new("Alice".to_string(), 30);
    println!("{}", user.greet());
}
