//! Resolved spell capabilities for one bot
//! (`BotSpellKit`): the highest known rank per
//! capability, as spell ids (`None` when the C++
//! kit returns `nullptr`). The per-class resolution
//! tables land with the behaviours (Slice E); the
//! host fills this struct from the bot's spellbook.

/// Capability slots of `BotSpellKit` plus the
/// classified spellbook behaviours read union
/// fields from (`m_ai->m_spells` in C++, reached
/// through the kit's AI pointer there).
#[derive(Clone, Debug, Default)]
pub struct SpellKit {
    pub class_id: u8,
    pub spells: std::sync::Arc<crate::class_spells::ClassSpells>,
    pub interrupt: Option<u32>,
    pub interrupt_alt: Option<u32>,
    pub filler: Option<u32>,
    pub execute_ability: Option<u32>,
    pub aoe_spell: Option<u32>,
    pub dot_spells: Vec<u32>,
    pub crowd_control: Option<u32>,
    pub root: Option<u32>,
    pub threat_dump: Option<u32>,
    pub self_buff: Option<u32>,
    pub party_buff: Option<u32>,
    pub taunt: Option<u32>,
    pub aoe_taunt: Option<u32>,
    pub emergency_ability: Option<u32>,
    pub escape_ability: Option<u32>,
}

impl SpellKit {
    /// An empty kit for a class (nothing learned).
    #[must_use]
    pub fn new(class_id: u8) -> Self {
        Self {
            class_id,
            ..Self::default()
        }
    }

    /// Resolves capabilities from a classified
    /// spellbook (`BotSpellKit` getters). `form` is
    /// the `ShapeshiftForm` byte (druid filler);
    /// recompute when form or role changes like the
    /// C++ live view.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "straight transcription of the 15 BotSpellKit getters"
    )]
    pub fn resolve(
        spells: &std::sync::Arc<crate::class_spells::ClassSpells>,
        is_tank: bool,
        form: u8,
    ) -> Self {
        let g = |f: &str| spells.get(f);
        let first = |fields: &[&str]| fields.iter().find_map(|f| g(f));
        let class_id = spells.class_id;
        let mut kit = Self::new(class_id);
        kit.spells = std::sync::Arc::clone(spells);
        kit.interrupt = match class_id {
            1 => g("pPummel"),
            4 => g("pKick"),
            8 => g("pCounterspell"),
            7 => g("pEarthShock"),
            5 => g("pSilence"),
            11 => g("pBash"),
            _ => None,
        };
        kit.interrupt_alt = match class_id {
            1 => g("pShieldBash"),
            4 => g("pGouge"),
            3 => g("pConcussiveShot"),
            _ => None,
        };
        kit.filler = match class_id {
            1 if is_tank => first(&["pShieldSlam", "pRevenge", "pSunderArmor"]),
            1 => first(&["pBloodthirst", "pMortalStrike", "pHeroicStrike"]),
            4 => first(&["pBackstab", "pSinisterStrike"]),
            3 => first(&["pAimedShot", "pArcaneShot"]),
            8 => first(&["pFrostbolt", "pFireball", "pScorch"]),
            5 => first(&["pMindFlay", "pMindBlast", "pSmite"]),
            9 => g("pShadowBolt"),
            2 if is_tank => first(&["pHolyShield", "pConsecration"]),
            2 => g("pJudgement"),
            7 => first(&["pStormstrike", "pLightningBolt"]),
            11 => match form {
                0x01 => first(&["pShred", "pClaw"]), // FORM_CAT
                0x05 | 0x08 => g("pMaul"),           // BEAR / DIREBEAR
                _ => g("pWrath"),
            },
            _ => None,
        };
        kit.execute_ability = match class_id {
            1 => g("pExecute"),
            9 => g("pShadowburn"),
            2 => g("pHammerOfWrath"),
            _ => None,
        };
        kit.aoe_spell = match class_id {
            1 => g("pWhirlwind"),
            8 => first(&["pBlizzard", "pArcaneExplosion"]),
            9 => g("pRainOfFire"),
            5 => g("pHolyNova"),
            2 => g("pConsecration"),
            3 => g("pVolley"),
            11 => g("pHurricane"),
            7 => g("pChainLightning"),
            _ => None,
        };
        let dot_fields: &[&str] = match class_id {
            9 => &["pCorruption", "pImmolate", "pSiphonLife"],
            5 => &["pShadowWordPain", "pDevouringPlague"],
            11 => &["pMoonfire", "pInsectSwarm", "pRake"],
            3 => &["pSerpentSting"],
            7 => &["pFlameShock"],
            4 => &["pGarrote", "pRupture"],
            1 => &["pRend"],
            _ => &[],
        };
        kit.dot_spells = dot_fields.iter().filter_map(|f| g(f)).collect();
        kit.crowd_control = match class_id {
            2 => g("pHammerOfJustice"),
            8 => g("pPolymorph"),
            5 => g("pShackleUndead"),
            9 => g("pBanish"),
            4 => g("pBlind"),
            11 => g("pHibernate"),
            _ => None,
        };
        kit.root = match class_id {
            8 => g("pFrostNova"),
            11 => g("pEntanglingRoots"),
            _ => None,
        };
        kit.threat_dump = match class_id {
            4 => g("pFeint"),
            5 => g("pFade"),
            11 => g("pCower"),
            3 => g("pFeignDeath"),
            _ => None,
        };
        kit.self_buff = match class_id {
            8 => g("pIceArmor"),
            9 => g("pDemonArmor"),
            5 => g("pInnerFire"),
            7 => g("pLightningShield"),
            1 => g("pBattleShout"),
            _ => None,
        };
        kit.party_buff = match class_id {
            8 => first(&["pArcaneBrilliance", "pArcaneIntellect"]),
            5 => first(&["pPrayerofFortitude", "pPowerWordFortitude"]),
            11 => first(&["pGiftoftheWild", "pMarkoftheWild"]),
            _ => None,
        };
        kit.taunt = spells.taunts.first().copied();
        kit.aoe_taunt = match class_id {
            1 => g("pChallengingShout"),
            11 => g("pChallengingRoar"),
            _ => None,
        };
        kit.emergency_ability = match class_id {
            8 => g("pIceBlock"),
            1 => first(&["pLastStand", "pShieldWall"]),
            2 => g("pDivineShield"),
            4 => g("pVanish"),
            3 => g("pFeignDeath"),
            9 => g("pDeathCoil"),
            5 => g("pPowerWordShield"),
            11 => g("pBarkskin"),
            _ => None,
        };
        kit.escape_ability = match class_id {
            8 => g("pBlink"),
            4 => g("pSprint"),
            11 => g("pDash"),
            3 => g("pDisengage"),
            _ => None,
        };
        kit
    }
}
