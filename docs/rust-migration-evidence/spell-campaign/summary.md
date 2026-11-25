# Spell campaign summary

## Druid (6 rows)
counts: BLOCKED=1, FAIL=1, PASS=4
bucket counts: corpse/BLOCKED=1, form/FAIL=1, form/PASS=2, friendly/PASS=1, hostile_dummy/PASS=1
- 6795 Growl rank 1/1 form target=17379390973850681688: FAIL CAST_RESULT(86) prepare expected_failure=True | expected wrong-form rejection for Growl: CAST_RESULT(86) after 1 attempt(s)
- 20484 Rebirth rank 1/5 corpse target=900015: BLOCKED wire-target-unsupported  expected_failure=False | corpse-mask cast: no bound SPELL_GO or CAST_RESULT (parser discards corpse GUIDs); socket alive=True

## Mage (2 rows)
counts: PASS=2
bucket counts: hostile_dummy/PASS=1, self/PASS=1

## Warrior (2 rows)
counts: PASS=2
bucket counts: hostile_dummy/PASS=1, self/PASS=1
## Druid run-class (63 rows)

counts: BLOCKED=19, FAIL=4, PASS=25, PASSIVE-VERIFIED=15
- 2782 Remove Curse friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 8946 Cure Poison friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 9830 Shred form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9850 Claw form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9867 Ravage form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9881 Maul form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9898 Demoralizing Roar form_totem target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState; form 9634 setup verified, exited=True
- 9901 Soothe Animal hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 9904 Rake form_totem target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState; form 768 setup verified, exited=True
- 9907 Faerie Fire hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 9908 Swipe form_totem target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState; form 9634 setup verified, exited=True
- 9910 Thorns friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False
- 9912 Wrath hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 17392 Faerie Fire (Feral) form_totem target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState; form 768 setup verified, exited=True
- 17402 Hurricane ground_area target=None: BLOCKED dummy-unavailable  | no live dummy for the radius leg
- 18658 Hibernate hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 20748 Rebirth corpse target=None: BLOCKED wire-target-unsupported  | no corpse-target scenario in a live class run; the parser discards corpse GUIDs (correction 2)
- 21850 Gift of the Wild item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17026 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 24977 Insect Swarm hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 25297 Healing Touch friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 25298 Starfire hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 25299 Rejuvenation friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False
- 29166 Innervate friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False

failure clusters:
- phase=prepare reason=CAST_RESULT(None/89) bucket=friendly count=3 spells=[9910, 25299, 29166]
- phase=prepare reason=CAST_RESULT(89) bucket=friendly count=1 spells=[25297]
## Druid run-class (63 rows)

counts: BLOCKED=8, PASS=40, PASSIVE-VERIFIED=15
- 2782 Remove Curse friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 8946 Cure Poison friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 9830 Shred form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9850 Claw form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9867 Ravage form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 9881 Maul form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 20748 Rebirth corpse target=None: BLOCKED wire-target-unsupported  | no corpse-target scenario in a live class run; the parser discards corpse GUIDs (correction 2)
- 21850 Gift of the Wild item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17026 absent from bags and the engine has no reagent gate; scenario-inapplicable
## Warlock run-class (113 rows)

counts: BLOCKED=37, PASS=62, PASSIVE-VERIFIED=14
- 691 Summon Felhunter item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 693 Create Soulstone (Minor) item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 697 Summon Voidwalker item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 712 Summon Succubus item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 1098 Enslave Demon item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 1454 Life Tap self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 2362 Create Spellstone item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 2947 Fire Shield party_duel target=None: BLOCKED no-party  | solo character with no party; scenario-inapplicable
- 3716 Torment hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 5857 Hellfire Effect ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 6201 Create Healthstone (Minor) item_reagent_ammo target=None: BLOCKED script-evidence-unknown  | Effect SCRIPT_EFFECT: behavior lives in spell scripts, not SpellEffects; classify_only until G4 script evidence (script-evidence-unknown)
- 6353 Soul Fire item_reagent_ammo target=None: BLOCKED fixture-gear  | offhand is not armor-class (class 4): None; starter lacks a shield
- 6360 Soothing Kiss hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 6366 Create Firestone (Lesser) item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 11681 Hellfire Effect ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 11682 Hellfire Effect ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 11689 Life Tap self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 11726 Enslave Demon item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 11730 Create Healthstone (Major) item_reagent_ammo target=None: BLOCKED script-evidence-unknown  | Effect SCRIPT_EFFECT: behavior lives in spell scripts, not SpellEffects; classify_only until G4 script evidence (script-evidence-unknown)
- 11771 Fire Shield party_duel target=None: BLOCKED no-party  | solo character with no party; scenario-inapplicable
- 11775 Torment hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 11785 Soothing Kiss hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 17728 Create Spellstone (Major) item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 17735 Suffering ground_area target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 17752 Suffering ground_area target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 17924 Soul Fire item_reagent_ammo target=None: BLOCKED fixture-gear  | offhand is not armor-class (class 4): None; starter lacks a shield
- 17953 Create Firestone (Major) item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 18540 Ritual of Doom item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 16583 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 18867 Shadowburn item_reagent_ammo target=None: BLOCKED fixture-gear  | offhand is not armor-class (class 4): None; starter lacks a shield
- 18871 Shadowburn item_reagent_ammo target=None: BLOCKED fixture-gear  | offhand is not armor-class (class 4): None; starter lacks a shield
- 18937 Dark Pact pet target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 18938 Dark Pact pet target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 19244 Spell Lock hostile_dummy target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 19505 Devour Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 19647 Spell Lock hostile_dummy target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 19736 Devour Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 20757 Create Soulstone (Major) item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 6265 absent from bags and the engine has no reagent gate; scenario-inapplicable
## Hunter run-class (80 rows)

counts: BLOCKED=14, PASS=53, PASSIVE-VERIFIED=13
- 781 Disengage rank 1/3 item_reagent_ammo target=None: BLOCKED effect-marker-only  expected_failure=False | threat-only: no wire-observable outcome; classify_only
- 982 Revive Pet rank 1/1 ground_area target=None: BLOCKED target-not-dead  expected_failure=False | no dead-pet scenario in a live class run; revive rows stay uncast
- 1499 Freezing Trap rank 1/3 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13795 Immolation Trap rank 1/5 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13809 Frost Trap rank 1/1 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13813 Explosive Trap rank 1/3 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14273 Disengage rank 3/3 item_reagent_ammo target=None: BLOCKED effect-marker-only  expected_failure=False | threat-only: no wire-observable outcome; classify_only
- 14305 Immolation Trap rank 5/5 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14311 Freezing Trap rank 3/3 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14317 Explosive Trap rank 3/3 ground_area target=None: BLOCKED effect-marker-only  expected_failure=False | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 15632 Distracting Shot rank 6/6 item_reagent_ammo target=None: BLOCKED effect-marker-only  expected_failure=False | threat-only: no wire-observable outcome; classify_only
- 19801 Tranquilizing Shot rank 1/1 item_reagent_ammo target=None: BLOCKED effect-marker-only  expected_failure=False | dispel needs a debuffed ally; classify_only
- 20736 Distracting Shot rank 1/6 item_reagent_ammo target=None: BLOCKED effect-marker-only  expected_failure=False | threat-only: no wire-observable outcome; classify_only
- 24406 Improved Mend Pet rank 1/1 pet target=None: BLOCKED effect-marker-only  expected_failure=False | dispel needs a debuffed ally; classify_only
## Hunter run-class (3 rows)

counts: PASS=3
## Hunter run-class (80 rows)

counts: BLOCKED=14, PASS=53, PASSIVE-VERIFIED=13
- 781 Disengage item_reagent_ammo target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 982 Revive Pet ground_area target=None: BLOCKED target-not-dead  | no dead-pet scenario in a live class run; revive rows stay uncast
- 1499 Freezing Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13795 Immolation Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13809 Frost Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 13813 Explosive Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14273 Disengage item_reagent_ammo target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 14305 Immolation Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14311 Freezing Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 14317 Explosive Trap ground_area target=None: BLOCKED effect-marker-only  | no wire-observable outcome for this row shape in a live run (bare trap placements, marker-only rows); left uncast
- 15632 Distracting Shot item_reagent_ammo target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 19801 Tranquilizing Shot item_reagent_ammo target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 20736 Distracting Shot item_reagent_ammo target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 24406 Improved Mend Pet pet target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
## Shaman run-class (93 rows)

counts: BLOCKED=51, PASS=27, PASSIVE-VERIFIED=15
- 370 Purge hostile_dummy target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 526 Cure Poison friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1535 Fire Nova Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 2008 Ancestral Spirit corpse target=None: BLOCKED wire-target-unsupported  | no corpse-target scenario in a live class run; the parser discards corpse GUIDs (correction 2)
- 2484 Earthbind Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 2870 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 3599 Searing Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 5394 Healing Stream Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 5675 Mana Spring Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 5730 Stoneclaw Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 6495 Sentry Totem form_totem target=None: BLOCKED engine-totem  | totem/stance hardware scenario not implemented in this harness; engine-totem
- 8012 Purge hostile_dummy target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 8071 Stoneskin Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8075 Strength of Earth Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8143 Tremor Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8166 Poison Cleansing Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8170 Disease Cleansing Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8177 Grounding Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8181 Frost Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8184 Fire Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8190 Magma Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8227 Flametongue Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8232 Windfury Weapon self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8512 Windfury Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 8835 Grace of Air Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10408 Stoneskin Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10428 Stoneclaw Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10438 Searing Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10463 Healing Stream Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10479 Frost Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10497 Mana Spring Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10538 Fire Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10587 Magma Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10595 Nature Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10601 Nature Resistance Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 10614 Windfury Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 11315 Fire Nova Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 15107 Windwall Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 15112 Windwall Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 16268 Parry self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 16362 Windfury Weapon self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 16387 Flametongue Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 17354 Mana Tide Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 17359 Mana Tide Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 20777 Ancestral Spirit corpse target=None: BLOCKED wire-target-unsupported  | no corpse-target scenario in a live class run; the parser discards corpse GUIDs (correction 2)
- 21169 Reincarnation item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17030 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 24844 Lightning Breath hostile_dummy target=None: BLOCKED no-pet  | no pet scenario for this class in a live run; nothing to observe
- 25012 Lightning Breath hostile_dummy target=None: BLOCKED no-pet  | no pet scenario for this class in a live run; nothing to observe
- 25359 Grace of Air Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 25361 Strength of Earth Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 25908 Tranquil Air Totem form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
## Warrior run-class (85 rows)

counts: BLOCKED=12, PASS=54, PASSIVE-VERIFIED=19
- 100 Charge form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 1680 Whirlwind form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 2687 Bloodrage self target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 5308 Execute form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 7384 Overpower form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 11578 Charge form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 11585 Overpower form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 20252 Intercept form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 20617 Intercept form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 20662 Execute form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 26090 Thunderstomp ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 26188 Thunderstomp ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
## Rogue run-class (59 rows)

counts: BLOCKED=10, PASS=34, PASSIVE-VERIFIED=15
- 921 Pick Pocket form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 1725 Distract ground_area target=None: BLOCKED effect-marker-only  | facing/motion only (EffectDistract): no wire-observable outcome; classify_only
- 1842 Disarm Trap form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 1856 Vanish item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 5140 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 1857 Vanish item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 5140 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 1966 Feint hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
- 2094 Blind item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 5530 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8676 Ambush form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 11269 Ambush form_totem target=None: BLOCKED effect-marker-only  | EffectEmpty/EffectUnused/marker-only: no server-side change; classify_only
- 25302 Feint hostile_dummy target=None: BLOCKED effect-marker-only  | threat-only: no wire-observable outcome; classify_only
## Priest run-class (113 rows)

counts: BLOCKED=25, FAIL=14, PASS=61, PASSIVE-VERIFIED=13
- 17 Power Word: Shield party_duel target=None: BLOCKED no-party  | solo character with no party; scenario-inapplicable
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 2050 Lesser Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2053 Lesser Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2054 Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2060 Greater Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2061 Flash Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2096 Mind Vision self target=900021: FAIL CAST_RESULT(9) prepare | no bound SPELL_GO for spell 2096 (reason=9)
- 2943 Touch of Weakness hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 2944 Devouring Plague hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 6064 Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 6346 Fear Ward friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False
- 8092 Mind Blast hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 8122 Psychic Scream ground_area target=None: BLOCKED dummy-unavailable  | no live dummy for the radius leg
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 9035 Hex of Weakness hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 9484 Shackle Undead hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10797 Starshards hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10890 Psychic Scream ground_area target=None: BLOCKED dummy-unavailable  | no live dummy for the radius leg
- 10894 Shadow Word: Pain hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10901 Power Word: Shield party_duel target=None: BLOCKED no-party  | solo character with no party; scenario-inapplicable
- 10909 Mind Vision self target=900021: FAIL CAST_RESULT(9) prepare | no bound SPELL_GO for spell 10909 (reason=9)
- 10912 Mind Control hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10917 Flash Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 10934 Smite hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 19266 Touch of Weakness self target=900021: FAIL  apply | SPELL_GO without required observable outcome: caster_aura=False
- 20770 Resurrection corpse target=None: FAIL  prepare | no live bear to kill the victim ally
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 27870 Lightwell ground_area target=17379390973850682870: FAIL  apply | SPELL_GO without required observable outcome: object_seen=False, health 100->100, matching damage=0
- 27871 Lightwell ground_area target=17379390973850682869: FAIL CAST_RESULT(60) prepare | no bound SPELL_GO for spell 27871 (reason=60)

failure clusters:
- phase=prepare reason=CAST_RESULT(89) bucket=friendly count=7 spells=[2050, 2053, 2054, 2060, 2061, 6064, 10917]
- phase=prepare reason=CAST_RESULT(9) bucket=self count=2 spells=[2096, 10909]
- phase=apply reason=None bucket=ground_area count=1 spells=[27870]
- phase=apply reason=None bucket=self count=1 spells=[19266]
- phase=prepare reason=CAST_RESULT(60) bucket=ground_area count=1 spells=[27871]
- phase=prepare reason=CAST_RESULT(None/89) bucket=friendly count=1 spells=[6346]
- phase=prepare reason=None bucket=corpse count=1 spells=[20770]
## Priest run-class (113 rows)

counts: BLOCKED=23, FAIL=10, PASS=67, PASSIVE-VERIFIED=13
- 17 Power Word: Shield friendly target=900015: FAIL CAST_RESULT(97/97) prepare | blessing legs incomplete: self leg go=False reason=97 aura=False; ally leg go=False reason=97 aura=False
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 2050 Lesser Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2053 Lesser Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2054 Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2060 Greater Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2061 Flash Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 2096 Mind Vision self target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 2943 Touch of Weakness hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 2944 Devouring Plague hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 6064 Heal friendly target=900015: FAIL CAST_RESULT(89) prepare | no bound SPELL_GO (reason=89)
- 6346 Fear Ward friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False
- 8092 Mind Blast hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 8122 Psychic Scream ground_area target=None: BLOCKED dummy-unavailable  | no live dummy for the radius leg
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 9035 Hex of Weakness hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 9484 Shackle Undead hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10797 Starshards hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10890 Psychic Scream ground_area target=None: BLOCKED dummy-unavailable  | no live dummy for the radius leg
- 10894 Shadow Word: Pain hostile_dummy target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 10901 Power Word: Shield friendly target=900015: FAIL CAST_RESULT(None/89) prepare | blessing legs incomplete: self leg go=True reason=None aura=True; ally leg go=False reason=89 aura=False
- 10909 Mind Vision self target=None: BLOCKED dummy-unavailable  | no live 705/707 counter left in WorldState
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 27871 Lightwell ground_area target=17379390973850682869: FAIL  apply | SPELL_GO without required observable outcome: object_seen=False, health 100->100, matching damage=0

failure clusters:
- phase=prepare reason=CAST_RESULT(89) bucket=friendly count=6 spells=[2050, 2053, 2054, 2060, 2061, 6064]
- phase=prepare reason=CAST_RESULT(None/89) bucket=friendly count=2 spells=[6346, 10901]
- phase=apply reason=None bucket=ground_area count=1 spells=[27871]
- phase=prepare reason=CAST_RESULT(97/97) bucket=friendly count=1 spells=[17]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=1, PASS=87, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 19266 Touch of Weakness self target=900021: FAIL  apply | SPELL_GO without required observable outcome: caster_aura=False
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op

failure clusters:
- phase=apply reason=None bucket=self count=1 spells=[19266]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=1, PASS=87, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 20770 Resurrection corpse target=None: FAIL  prepare | no live bear to kill the victim ally
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op

failure clusters:
- phase=prepare reason=None bucket=corpse count=1 spells=[20770]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=2, PASS=86, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 2006 Resurrection corpse target=None: FAIL  prepare | no live bear or trogg to kill the victim ally
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 20770 Resurrection corpse target=None: FAIL  prepare | no live bear or trogg to kill the victim ally
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op

failure clusters:
- phase=prepare reason=None bucket=corpse count=2 spells=[2006, 20770]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=2, PASS=86, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8122 Psychic Scream ground_area target=17379390973850682378: FAIL  apply | SPELL_GO without required observable outcome: target_aura=False
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 19280 Devouring Plague hostile_dummy target=17379390973850682378: FAIL  apply | SPELL_GO without required observable outcome: target_aura=False after 2 attempt(s); landed cast retained over cooldown-blocked recast
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op

failure clusters:
- phase=apply reason=None bucket=ground_area count=1 spells=[8122]
- phase=apply reason=None bucket=hostile_dummy count=1 spells=[19280]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=2, PASS=86, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 2006 Resurrection corpse target=None: FAIL  prepare | no live bear or trogg to kill the victim ally
- 8122 Psychic Scream ground_area target=17379390973850682871: FAIL  apply | SPELL_GO without required observable outcome: target_aura=False
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op

failure clusters:
- phase=apply reason=None bucket=ground_area count=1 spells=[8122]
- phase=prepare reason=None bucket=corpse count=1 spells=[2006]
## Priest run-class (113 rows)

counts: BLOCKED=12, FAIL=1, PASS=87, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 27870 Lightwell ground_area target=17379390973850682870: FAIL  apply | SPELL_GO without required observable outcome: object_seen=False, health 100->100, matching damage=0

failure clusters:
- phase=apply reason=None bucket=ground_area count=1 spells=[27870]
## Priest run-class (113 rows)

counts: BLOCKED=12, PASS=88, PASSIVE-VERIFIED=13
- 527 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 528 Cure Disease friendly target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 988 Dispel Magic self target=None: BLOCKED effect-marker-only  | dispel needs a debuffed ally; classify_only
- 1706 Levitate item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17056 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 8129 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 10876 Mana Burn hostile_dummy target=None: BLOCKED effect-marker-only  | sub-observable heal (raw max 0 < 10, below one regen tick); classify_only
- 15430 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
- 21562 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17028 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 21564 Prayer of Fortitude item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27681 Prayer of Spirit item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27683 Prayer of Shadow Protection item_reagent_ammo target=None: BLOCKED fixture-reagent  | reagent 17029 absent from bags and the engine has no reagent gate; scenario-inapplicable
- 27801 Holy Nova ground_area target=None: BLOCKED effect-marker-only  | trigger-only applier: direct cast is a fork-exact no-op
