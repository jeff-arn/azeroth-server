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
