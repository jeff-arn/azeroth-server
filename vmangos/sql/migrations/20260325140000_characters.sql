DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260325140000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260325140000');
-- Add your query below.

-- Guild bot roster metadata (extends characters table with spec/role info)
CREATE TABLE IF NOT EXISTS `guild_bot_roster` (
  `guid` int(11) unsigned NOT NULL COMMENT 'Character GUID from characters table',
  `spec` varchar(32) NOT NULL DEFAULT '' COMMENT 'Spec name (e.g. protection, holy, feral_bear)',
  `role` enum('tank','healer','dps') NOT NULL DEFAULT 'dps',
  `raid_assigned` tinyint(1) unsigned NOT NULL DEFAULT 0 COMMENT 'Selected for next raid',
  `active` tinyint(1) unsigned NOT NULL DEFAULT 1 COMMENT 'Available for summoning',
  `gear_score` float NOT NULL DEFAULT 0 COMMENT 'Cached gear score for display',
  PRIMARY KEY (`guid`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8;

-- Stat weights for spec-aware upgrade evaluation
CREATE TABLE IF NOT EXISTS `guild_bot_spec_weights` (
  `class` tinyint(3) unsigned NOT NULL COMMENT 'Class ID (1=Warrior, 2=Paladin, etc.)',
  `spec` varchar(32) NOT NULL COMMENT 'Spec name matching guild_bot_roster.spec',
  `stamina` float NOT NULL DEFAULT 0,
  `strength` float NOT NULL DEFAULT 0,
  `agility` float NOT NULL DEFAULT 0,
  `intellect` float NOT NULL DEFAULT 0,
  `spirit` float NOT NULL DEFAULT 0,
  `attack_power` float NOT NULL DEFAULT 0,
  `spell_power` float NOT NULL DEFAULT 0,
  `healing_power` float NOT NULL DEFAULT 0,
  `hit_rating` float NOT NULL DEFAULT 0,
  `crit_rating` float NOT NULL DEFAULT 0,
  `defense` float NOT NULL DEFAULT 0,
  `dodge` float NOT NULL DEFAULT 0,
  `parry` float NOT NULL DEFAULT 0,
  `block` float NOT NULL DEFAULT 0,
  `mp5` float NOT NULL DEFAULT 0,
  PRIMARY KEY (`class`, `spec`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8;

-- Create the guild (id=1, tabard: gold lion on blue background)
INSERT IGNORE INTO `guild` (`guild_id`, `name`, `leader_guid`, `emblem_style`, `emblem_color`, `border_style`, `border_color`, `background_color`, `info`, `motd`, `create_date`)
VALUES (1, 'Vanguard of Azeroth', 200, 35, 17, 1, 17, 45, 'A guild of stalwart heroes.', 'Ready for the next raid!', UNIX_TIMESTAMP());

-- Create guild ranks
INSERT IGNORE INTO `guild_rank` (`guild_id`, `id`, `name`, `rights`)
VALUES
(1, 0, 'Guild Master', 1048575),
(1, 1, 'Officer', 1048575),
(1, 2, 'Raider', 590145),
(1, 3, 'Member', 590145),
(1, 4, 'Initiate', 65);

-- Create 20 bot characters (account 98, GUIDs 200-219)
-- Faction: Alliance (Human=1, Dwarf=3, NightElf=4, Gnome=7)
-- The roster is faction-flexible: swap races to Horde equivalents to change faction.

-- Tanks (2)
INSERT IGNORE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`, `money`)
VALUES
(200, 98, 'Aldric',    1, 1, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Prot Warrior
(201, 98, 'Thornbark', 4, 11, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- NightElf Feral Druid

-- Healers (5)
(202, 98, 'Elara',     1, 5, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Holy Priest
(203, 98, 'Brinley',   3, 5, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Dwarf Holy Priest
(204, 98, 'Garrick',   1, 2, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Holy Paladin
(205, 98, 'Dara',      3, 2, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Dwarf Holy Paladin
(206, 98, 'Sylvara',   4, 11, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- NightElf Resto Druid

-- Melee DPS (5)
(207, 98, 'Rodrik',    1, 1, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Fury Warrior
(208, 98, 'Baelgar',   3, 1, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Dwarf Fury Warrior
(209, 98, 'Cael',      1, 4, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Combat Rogue
(210, 98, 'Fizzik',    7, 4, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Gnome Combat Rogue
(211, 98, 'Haldren',   1, 2, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Ret Paladin

-- Ranged DPS (8)
(212, 98, 'Aelindra',  4, 3, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- NightElf MM Hunter
(213, 98, 'Torbin',    3, 3, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Dwarf MM Hunter
(214, 98, 'Nimble',    7, 8, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Gnome Frost Mage
(215, 98, 'Maelis',    1, 8, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Frost Mage
(216, 98, 'Wexli',     7, 9, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Gnome Warlock
(217, 98, 'Sevara',    1, 9, 1, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Warlock
(218, 98, 'Davan',     1, 5, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0),  -- Human Shadow Priest
(219, 98, 'Feranor',   4, 11, 0, 60, 1519, 0, -8949.95, -132.493, 83.5312, 0, 0); -- NightElf Feral Cat Druid

-- Add all bots to guild as Raiders (rank 2), leader as Guild Master (rank 0)
INSERT IGNORE INTO `guild_member` (`guild_id`, `guid`, `rank`, `player_note`, `officer_note`)
VALUES
(1, 200, 0, 'Main Tank', ''),
(1, 201, 2, 'Offtank / Bear', ''),
(1, 202, 2, 'Healer', ''),
(1, 203, 2, 'Healer', ''),
(1, 204, 2, 'Healer', ''),
(1, 205, 2, 'Healer', ''),
(1, 206, 2, 'Healer', ''),
(1, 207, 2, 'DPS', ''),
(1, 208, 2, 'DPS', ''),
(1, 209, 2, 'DPS', ''),
(1, 210, 2, 'DPS', ''),
(1, 211, 2, 'DPS', ''),
(1, 212, 2, 'DPS', ''),
(1, 213, 2, 'DPS', ''),
(1, 214, 2, 'DPS', ''),
(1, 215, 2, 'DPS', ''),
(1, 216, 2, 'DPS', ''),
(1, 217, 2, 'DPS', ''),
(1, 218, 2, 'DPS', ''),
(1, 219, 2, 'DPS', '');

-- Populate guild bot roster metadata
INSERT IGNORE INTO `guild_bot_roster` (`guid`, `spec`, `role`, `raid_assigned`, `active`)
VALUES
(200, 'protection',    'tank',   1, 1),
(201, 'feral_bear',    'tank',   1, 1),
(202, 'holy',          'healer', 1, 1),
(203, 'holy',          'healer', 1, 1),
(204, 'holy',          'healer', 1, 1),
(205, 'holy',          'healer', 1, 1),
(206, 'restoration',   'healer', 1, 1),
(207, 'fury',          'dps',    1, 1),
(208, 'fury',          'dps',    1, 1),
(209, 'combat',        'dps',    1, 1),
(210, 'combat',        'dps',    1, 1),
(211, 'retribution',   'dps',    1, 1),
(212, 'marksmanship',  'dps',    1, 1),
(213, 'marksmanship',  'dps',    1, 1),
(214, 'frost',         'dps',    1, 1),
(215, 'frost',         'dps',    1, 1),
(216, 'sm_ruin',       'dps',    1, 1),
(217, 'sm_ruin',       'dps',    1, 1),
(218, 'shadow',        'dps',    1, 1),
(219, 'feral_cat',     'dps',    1, 1);

-- Populate stat weights for all specs
-- Weights are relative: higher = more valuable for that spec
-- Used by upgrade evaluation to compare items

-- Warrior specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(1, 'protection',  1.5, 1.0, 0.5, 0.0, 0.0, 0.3, 0.0, 0.0, 1.0, 0.5, 2.0, 1.5, 1.5, 1.5, 0.0),
(1, 'fury',        0.5, 1.3, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Paladin specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(2, 'holy',        0.5, 0.0, 0.0, 1.5, 0.5, 0.0, 0.3, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.5),
(2, 'protection',  1.5, 0.8, 0.0, 0.5, 0.0, 0.0, 0.3, 0.0, 1.0, 0.0, 2.0, 1.5, 1.0, 1.5, 0.0),
(2, 'retribution', 0.5, 1.5, 0.5, 0.3, 0.0, 1.0, 0.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Hunter specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(3, 'marksmanship', 0.3, 0.0, 2.0, 0.3, 0.0, 1.5, 0.0, 0.0, 2.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Rogue specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(4, 'combat',      0.3, 1.0, 1.5, 0.0, 0.0, 1.2, 0.0, 0.0, 2.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Priest specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(5, 'holy',   0.5, 0.0, 0.0, 1.5, 1.0, 0.0, 0.3, 2.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 1.5),
(5, 'shadow', 0.5, 0.0, 0.0, 1.0, 0.5, 0.0, 2.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5);

-- Shaman specs (for future Horde roster)
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(7, 'restoration', 0.5, 0.0, 0.0, 1.5, 0.5, 0.0, 0.3, 2.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 1.5),
(7, 'enhancement', 0.3, 1.2, 1.0, 0.3, 0.0, 1.5, 0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0),
(7, 'elemental',   0.5, 0.0, 0.0, 1.0, 0.3, 0.0, 2.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5);

-- Mage specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(8, 'frost', 0.3, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 2.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Warlock specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(9, 'sm_ruin', 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 1.5, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0);

-- Druid specs
INSERT IGNORE INTO `guild_bot_spec_weights` VALUES
(11, 'restoration', 0.5, 0.0, 0.0, 1.5, 1.0, 0.0, 0.3, 2.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 1.5),
(11, 'feral_bear',  1.5, 1.0, 1.5, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 2.0, 2.0, 0.0, 0.0, 0.0),
(11, 'feral_cat',   0.3, 1.0, 2.0, 0.0, 0.0, 1.5, 0.0, 0.0, 2.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0),
(11, 'balance',     0.5, 0.0, 0.0, 1.0, 0.3, 0.0, 2.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5);

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
