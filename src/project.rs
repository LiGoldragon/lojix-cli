use horizon_lib::{ClusterProposal, Horizon, Viewpoint};

use crate::error::Result;

pub struct HorizonProjection {
    proposal: ClusterProposal,
    viewpoint: Viewpoint,
}

impl HorizonProjection {
    pub fn new(proposal: ClusterProposal, viewpoint: Viewpoint) -> Self {
        Self {
            proposal,
            viewpoint,
        }
    }

    pub fn project(&self) -> Result<Horizon> {
        self.proposal.project(&self.viewpoint).map_err(Into::into)
    }
}
