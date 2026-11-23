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
