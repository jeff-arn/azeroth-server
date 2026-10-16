# Character Save/Load Surface — Phase 0 transcription (main doc §5 + §14 #36)

Branch `rust-migration` @ `3589f72`. READ-ONLY inspection; all claims pinned to file:line.
Load holder enum: [Player.h:610-636](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.h).
Query texts: `LoginQueryHolder::Initialize()` in [CharacterHandler.cpp:65-103](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Handlers/CharacterHandler.cpp).
Save root: `Player::SaveToDB` at [Player.cpp:16229-16420](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp);
transaction opened [Player.cpp:16246](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp) (`BeginTransaction(GetGUIDLow())`),
committed [Player.cpp:16405](/Users/jeffarn/Programming/azeroth-server/vmangos/src/game/Objects/Player.cpp).

## LOAD — 22 holder slots in enum order

| # | Enum (Player.h) | Table(s) | Key col | Cols | Query text (CharacterHandler.cpp) | Consumer |
|---|---|---|---|---|---|---|
| 0 | LOADFROM | `characters` | `guid`=%u | 58 (idx 0–57) | :73-80 | `Player::LoadFromDB` [Player.cpp:14470](…/Objects/Player.cpp) |
| 1 | LOADGROUP | `group_member` | `member_guid` | 1 | :81 | `_LoadGroup` [Player.cpp:15710](…/Objects/Player.cpp), called :14646 |
| 2 | LOADBOUNDINSTANCES | `character_instance` LEFT JOIN `instance` | `guid` | 4 | :82 | `_LoadBoundInstances` [Player.cpp:15726](…/Objects/Player.cpp), called :14657 |
| 3 | LOADAURAS | `character_aura` | `guid` | 14 | :83 | `_LoadAuras` [Player.cpp:15208](…/Objects/Player.cpp), called :14862 |
| 4 | LOADSPELLS | `character_spell` | `guid` | 3 | :84 | `_LoadSpells` [Player.cpp:15692](…/Objects/Player.cpp), called :14868 |
| 5 | LOADQUESTSTATUS | `character_queststatus` | `guid` | 15 | :85 | `_LoadQuestStatus` [Player.cpp:15594](…/Objects/Player.cpp), called :14875 |
| 6 | LOADHONORCP | `character_honor_cp` | `guid` | 5 | :86 | `HonorMgr::Load` [HonorMgr.cpp:693](…/HonorMgr.cpp), called Player.cpp:14656 |
| 7 | LOADREPUTATION | `character_reputation` | `guid` | 3 | :87 | `ReputationMgr::LoadFromDB` [ReputationMgr.cpp:394](…/ReputationMgr.cpp), called Player.cpp:14878 |
| 8 | LOADINVENTORY | `character_inventory` JOIN `item_instance` | `character_inventory.guid` | 15 | :88 | `_LoadInventory` [Player.cpp:15333](…/Objects/Player.cpp), called :14881 |
| 9 | LOADITEMLOOT | `item_loot` | `owner_guid` | 4 | :89 | `_LoadItemLoot` [Player.cpp:15556](…/Objects/Player.cpp), called :14886 |
| 10 | LOADACTIONS | `character_action` | `guid` | 3 | :90 | `MasterPlayer::LoadActions` [MasterPlayer.cpp:358](…/Chat/MasterPlayer.cpp), called CharacterHandler.cpp:521 |
| 11 | LOADSOCIALLIST | `character_social` | `guid`, LIMIT 255 | 2 | :91 | `MasterPlayer::LoadSocial` [MasterPlayer.cpp:433](…/Chat/MasterPlayer.cpp), called CharacterHandler.cpp:522 |
| 12 | LOADHOMEBIND | `character_homebind` | `guid` | 5 | :92 | `_LoadHomeBind` [Player.cpp:15973](…/Objects/Player.cpp), called :14623 |
| 13 | LOADSPELLCOOLDOWNS | `character_spell_cooldown` | `guid` | 5 | :93 | `_LoadSpellCooldowns` [Player.cpp:3978](…/Objects/Player.cpp), called :14944 |
| 14 | LOADGUILD | `guild_member` | `guid` | 2 | :94 | `_LoadGuild` [Player.cpp:16018](…/Objects/Player.cpp), called :14660 |
| 15 | LOADBGDATA | `character_battleground_data` | `guid` | 7 | :95 | `_LoadBGData` [Player.cpp:14403](…/Objects/Player.cpp), called :14658 |
| 16 | LOADACCOUNTDATA | `character_account_data` | `guid`, per-char mask | 3 | :96 | `WorldSession::LoadAccountData` [WorldSession.cpp:878](…/Server/WorldSession.cpp), called CharacterHandler.cpp:550 |
| 17 | LOADSKILLS | `character_skills` | `guid` | 3 | :97 | `_LoadSkills` [Player.cpp:20285](…/Objects/Player.cpp), called :14854 |
| 18 | LOADMAILS | `mail` | `receiver_guid`, ORDER BY id DESC | 14 | :98 | `MasterPlayer::LoadMails` [MasterPlayer.cpp:260](…/Chat/MasterPlayer.cpp), called CharacterHandler.cpp:523 |
| 19 | LOADMAILEDITEMS | `mail_items` JOIN `item_instance` | `receiver_guid` | 14 | :99 | `MasterPlayer::LoadMailedItems` [MasterPlayer.cpp:207](…/Chat/MasterPlayer.cpp), called CharacterHandler.cpp:524 |
| 20 | BATTLEGROUND_DATA | (none — dead slot) | — | — | NO SetPQuery anywhere | NO TakeResult anywhere (only enum [Player.h:632](…/Objects/Player.h)) |
| 21 | FORGOTTEN_SKILLS | `character_forgotten_skills` | `guid` | 2 | :100 | `_LoadForgottenSkills` [Player.cpp:20370](…/Objects/Player.cpp), called :14857 |

### LOADFROM positional columns (58 cols, idx 0–57; read at Player.cpp:14495-14629)

0 guid, 1 account, 2 name, 3 race, 4 class, 5 gender, 6 level, 7 xp, 8 money,
9 skin, 10 face, 11 hair_style, 12 hair_color, 13 facial_hair, 14 bank_bag_slots,
15 character_flags, 16 position_x, 17 position_y, 18 position_z, 19 map, 20 orientation,
21 known_taxi_mask, 22 played_time_total, 23 played_time_level, 24 rest_bonus,
25 logout_time, 26 reset_talents_multiplier, 27 reset_talents_time, 28 transport_guid,
29 transport_x, 30 transport_y, 31 transport_z, 32 transport_o, 33 extra_flags,
34 stable_slots, 35 death_expire_time, 36 current_taxi_path, 37 honor_rank_points,
38 honor_highest_rank, 39 honor_standing, 40 honor_last_week_hk, 41 honor_last_week_cp,
42 honor_stored_hk, 43 honor_stored_dk, 44 watched_faction, 45 drunk, 46 health,
47–51 power1–power5, 52 explored_zones, 53 ammo_id, 54 action_bars,
55 world_phase_mask, 56 create_time, 57 instance.
Anchor points verified in code: `fields[15]` flags :14509, `fields[44]` watched faction :14596,
`fields[45]` drunk :14562, `fields[52]` explored :14544, `fields[53]` ammo :14599,
`fields[54]` action bars :14602, `fields[55]` world mask :14530.

### Other load column layouts (positional)

- INVENTORY (15): 0 creator_guid, 1 gift_creator_guid, 2 count, 3 duration, 4 charges,
  5 flags, 6 enchantments, 7 random_property_id, 8 durability, 9 text, 10 bag, 11 slot,
  12 item_guid, 13 item_id, 14 generated_loot. Item field map comment [Player.cpp:4446](…/Objects/Player.cpp);
  per-item read via `Item::LoadFromDB` [Item.cpp:372](…/Objects/Item.cpp) (its own 0–11 comment :374-375).
- MAILEDITEMS (14): same as inventory but 10 mail_id, 11 item_guid, 12 item_id,
  13 generated_loot; consumed [MasterPlayer.cpp:207-258](…/Chat/MasterPlayer.cpp)
  (stale-item cleanup writes `character_deleted_items` + DELETEs at :232-234).
- MAIL (14): 0 id … 13 has_items; consumed [MasterPlayer.cpp:260-300](…/Chat/MasterPlayer.cpp) (:274-287).
- AURAS (14): caster_guid, item_guid, spell, stacks, charges, base_points0-2,
  periodic_time0-2, max_duration, duration, effect_index_mask; re-applied by `LoadAura` [Player.cpp:15246](…/Objects/Player.cpp).
- QUESTSTATUS (15): quest, status, rewarded, explored, timer, mob_count1-4, item_count1-4,
  reward_choice.
- ITEMLOOT (4): guid, item_id, amount, property → `_LoadItemLoot` :15556.
- COOLDOWNS (5): spell, spell_expire_time, category, category_expire_time, item_id → :3978.
- BOUNDINSTANCES (4): id, permanent, map, reset_time → :15726 (deletes stale binds :15750/:15785, inserts :15809).
- BGDATA (7): instance_id, team, join_x/y/z/o, join_map → :14403.
- HOMEBIND (5): map, zone, position_x/y/z → :15973. Saved separately by UPDATE at [Player.cpp:20781](…/Objects/Player.cpp), NOT in SaveToDB.
- Pet (outside holder, sync at login): `Pet::LoadPet` [Player.cpp:15582](…/Objects/Player.cpp)
  → `Pet::LoadPetFromDB` [Pet.cpp:100](…/Objects/Pet.cpp); corpse handled in-memory by
  `Player::LoadCorpse` [Player.cpp:15317](…/Objects/Player.cpp) (no SELECT at login).

## SAVE — all tables written by / under Player::SaveToDB (in call order)

In-GUIDLow-transaction = between Begin [Player.cpp:16246](…/Objects/Player.cpp)
and Commit [Player.cpp:16405](…/Objects/Player.cpp). Bottleneck guard: bots return early
(:16236), teleports defer via DELAYED_SAVE_PLAYER (:16240-16244).

| Order | Table | Shape | In txn? | Site |
|---|---|---|---|---|
| 1 | `characters` | REPLACE, **61 binds** (61 cols, :16253-16270; binds :16272-16388, executed :16389) | yes | Player.cpp:16252 |
| 2 | `character_battleground_data` | DELETE + conditional INSERT (8 binds) | yes | `_SaveBGData` [Player.cpp:20696-20726](…/Objects/Player.cpp) |
| 3 | `character_inventory` + `item_instance` + `item_loot` | per-item INSERT (5 binds) / UPDATE (5) / DELETE; anti-dupe DELETE (3 binds, :16609); then `Item::SaveToDB` per item (below) | yes | `_SaveInventory` [Player.cpp:16561-16675](…/Objects/Player.cpp) |
| 4 | `character_queststatus` | INSERT (15 binds, :16699) / UPDATE (15, :16719) / DELETE | yes | `_SaveQuestStatus` [Player.cpp:16677-16743](…/Objects/Player.cpp) |
| 5 | `character_spell` | DELETE + INSERT (4 binds each, PExecute) | yes | `_SaveSpells` [Player.cpp:16812-16837](…/Objects/Player.cpp) |
| 6 | `character_spell_cooldown` | DELETE + INSERT (6 binds, :4064-4070) | yes | `_SaveSpellCooldowns` [Player.cpp:4042-4074](…/Objects/Player.cpp) |
| 7 | `character_aura` | DELETE + INSERT (15 binds, :16469-16498) | yes | `_SaveAuras` [Player.cpp:16456-16500](…/Objects/Player.cpp) |
| 8 | `character_skills` | INSERT / UPDATE / DELETE (PExecute) + REPLACE `character_forgotten_skills` (3 binds, :16805) | yes | `_SaveSkills` [Player.cpp:16745-16810](…/Objects/Player.cpp) |
| 9 | `character_reputation` | DELETE + INSERT per dirty faction (PExecute) | yes | `ReputationMgr::SaveToDB` [ReputationMgr.cpp:448](…/ReputationMgr.cpp), called Player.cpp:16398 |
| 10 | `character_honor_cp` | INSERT per STATE_NEW row (PExecute; static armory block commented out) | yes | `HonorMgr::Save` [HonorMgr.cpp:633](…/HonorMgr.cpp), called Player.cpp:16399 |
| 11 | `character_tutorial` (by account) | UPDATE (9 binds) or INSERT (9 binds) on TUTORIALDATA_CHANGED/NEW | yes | `WorldSession::SaveTutorialsData` [WorldSession.cpp:1005](…/Server/WorldSession.cpp), called Player.cpp:16403 |
| 12 | `character_stats` | DELETE + INSERT (29 binds: guid + 28 stats, :16853-16879) | NO — after commit, only on logout or if stats-save-every-save (:16409) | `_SaveStats` [Player.cpp:16841-16880](…/Objects/Player.cpp) |
| 13 | `characters` (money only) | UPDATE money (fast-save path `SaveInventoryAndGoldToDB`/`SaveGoldToDB`, own or joined txn) | caller txn | [Player.cpp:16425-16454](…/Objects/Player.cpp) |
| 14 | `character_pet` + `pet_aura` + `pet_spell` + `pet_spell_cooldown` | own nested transaction (:16462/16466-16566): `character_pet` DELETE + UPDATE-slot + INSERT (23 binds, :16516-16564) | NO — separate txn after commit | `Pet::SavePetToDB(PET_SAVE_AS_CURRENT)` [Pet.cpp:408-574](…/Objects/Pet.cpp), called Player.cpp:16414 |

`Item::SaveToDB` [Item.cpp:253-370](…/Objects/Item.cpp): `character_gifts` UPDATE (wrapped gifts, :263);
`item_instance` REPLACE (14 binds, new) / UPDATE (14 binds, changed) (:273-302);
`item_loot` DELETE + INSERTs per loot slot (5 binds, :322-362); full delete path
`DeleteAllFromDB` [Item.cpp:468-486](…/Objects/Item.cpp) clears `item_text`, `item_instance`,
`character_gifts`. Pet sub-saves: `pet_spell_cooldown` DELETE+INSERT(3) [Pet.cpp:1560-1590](…/Objects/Pet.cpp);
`pet_spell` DELETE+INSERT(3) [Pet.cpp:1612-1655](…/Objects/Pet.cpp);
`pet_aura` DELETE+INSERT(15) [Pet.cpp:1758](…/Objects/Pet.cpp).
Note: `HonorMgr::SaveStoredData` [HonorMgr.cpp:683](…/HonorMgr.cpp) UPDATEs `characters`
honor columns but is NOT called from SaveToDB (called from honor-update paths); the
`characters` REPLACE at step 1 already persists honor fields.
`Corpse::SaveToDB` [Corpse.cpp:110](…/Objects/Corpse.cpp) REPLACE INTO `corpse` fires on
death/creation ([Player.cpp:4822](…/Objects/Player.cpp)), not during SaveToDB.

### characters REPLACE bind count = 61

61 columns [Player.cpp:16253-16261](…/Objects/Player.cpp), 61 `?` [Player.cpp:16262-16270](…/Objects/Player.cpp),
61 adds: 16 identity/appearance (:16272-16289), 6 map/instance/pos (:16293-16298),
5 transport (:16311-16317), 5 taxi/online/played (:16321-16331), 8 rest/logout/talent/extra/stable/zone/death (:16333-16340),
7 honor (:16343-16349), 8 watched/drunk/health/powers (:16352-16362),
6 explored-string/equipment-string/ammo/actionbars/worldmask/createtime (:16364-16388).
Quirk: one `ostringstream ss` is reused without reset, so the equipment_cache bind
repeats the explored_zones prefix (:16364-16382).

## Mail / auction / social vs a save — a Player::SaveToDB triggers NONE of these

- Actions + mails save in a SEPARATE transaction via `MasterPlayer::SaveToDB`
  [MasterPlayer.cpp:55-61](…/Chat/MasterPlayer.cpp) (SaveActions + SaveMails), called only at
  logout [WorldSession.cpp:770](…/Server/WorldSession.cpp) — never from Player::SaveToDB:
  `character_action` INSERT/UPDATE/DELETE [MasterPlayer.cpp:382-427](…/Chat/MasterPlayer.cpp);
  `mail` UPDATE(7) [MasterPlayer.cpp:97](…/Chat/MasterPlayer.cpp), `mail_items`/`mail`/`item_instance`/`item_text`
  DELETEs on MAIL_STATE_DELETED [MasterPlayer.cpp:110-137](…/Chat/MasterPlayer.cpp).
- Social writes are immediate, not save-driven: `character_social` INSERT/UPDATE/DELETE
  [SocialMgr.cpp:71-101](…/SocialMgr.cpp).
- Mail send is immediate: `mail` + `mail_items` INSERTs in `MailDraft::SendMailTo`
  [Mail.cpp:288](…/Mail/Mail.cpp) (~:325-344); auction buyout/sale note path reuses it.
- Auction writes are immediate: `auction` INSERT `AuctionEntry::SaveToDB`
  [AuctionHouseMgr.cpp:848-854](…/AuctionHouse/AuctionHouseMgr.cpp), DELETE `DeleteFromDB` :844-846;
  bidder takeover UPDATEs `item_instance.owner_guid` :176.
- Group/guild membership, homebind, instance binds: read at login, written by their own
  event paths (bind/unbind, SetHomebind :20781, _LoadBoundInstances steady-state :15809) —
  no writes inside SaveToDB.

## Could not pin to a line

- `PLAYER_LOGIN_QUERY_BATTLEGROUND_DATA` (enum 20): no SetPQuery and no TakeResult anywhere
  in vmangos/src — dead slot, likely leftover; `character_battleground_data` load works via LOADBGDATA.
- `LoadMails`/`LoadMailedItems` exact row→Mail field mapping beyond fields[0-13] verified above;
  `checked`/`stationery` width casts (GetUInt32 vs GetUInt8) taken as written at MasterPlayer.cpp:284-285.
- Whether `m_itemUpdateQueue` coalescing can skip `Item::SaveToDB` for ITEM_CHANGED rows is
  control-flow dependent (test!=item branch :16619-16625); statement shapes above cover all branches.
