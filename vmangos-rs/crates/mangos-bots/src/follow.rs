//! Formation-point follow (Slice D exemplar,
//! expressed over the formation fields of the
//! situation snapshot like the fork's formation
//! pre-step; `BehaviorPosition` supersedes it in
//! the ported sets).

use crate::behavior::BotBehavior;
use crate::context::SituationContext;
use crate::host::BotHost;
use crate::spell_kit::SpellKit;

/// Moves to the formation target whenever the gap
/// exceeds `tolerance` (plus hysteresis at score
/// time so a settled bot doesn't re-path).
pub struct FollowLeader {
    pub tolerance: f32,
}

impl BotBehavior for FollowLeader {
    fn name(&self) -> &'static str {
        "FollowLeader"
    }

    fn score(&self, ctx: &SituationContext, _host: &dyn BotHost) -> f32 {
        if ctx.has_formation_target && ctx.dist_to_formation_pos > self.tolerance + 2.0 {
            40.0
        } else {
            0.0
        }
    }

    fn execute(&mut self, ctx: &SituationContext, _kit: &SpellKit, host: &mut dyn BotHost) -> bool {
        if ctx.has_formation_target && ctx.dist_to_formation_pos > self.tolerance {
            host.move_point(
                ctx.formation_target_x,
                ctx.formation_target_y,
                ctx.formation_target_z,
            );
            true
        } else {
            false
        }
    }
}
