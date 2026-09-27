// Candidate generation, hill climbing algorithm, batch evaluation, and video temporal coherence

pub mod candidate_generator;
pub mod hill_climber;
pub mod video_evolution;
pub mod video_pipeline;

pub use candidate_generator::CandidateGenerator;
pub use hill_climber::{HillClimber, select_best};
pub use video_evolution::VideoEvolution;
pub use video_pipeline::VideoPipeline;
