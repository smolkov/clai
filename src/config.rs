use serte::{Serialize,Deserialize};


#[derive(Serialize,Deserialize,Debug)]
struct Config {
	pub api_key: String,
}