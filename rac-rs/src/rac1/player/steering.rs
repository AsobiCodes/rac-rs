//! Hero steering: empty apart from two empty test modules (`spring_tests`, `state_row_tests`).
//! Leftover from the retired hand port; still compiled because the old `Game` and the viewer still use the player types. Do not extend.






#[cfg(test)]
mod spring_tests {
	use super::*;
	fn near(x:f32,y:f32) { assert!((x-y).abs()<1e-6,"{x} != {y}"); }
}

#[cfg(test)]
mod state_row_tests {
	use super::*;

}
