





#[cfg(test)]
mod spring_tests {
	use super::*;
	fn near(x:f32,y:f32) { assert!((x-y).abs()<1e-6,"{x} != {y}"); }
}

#[cfg(test)]
mod state_row_tests {
	use super::*;

}
