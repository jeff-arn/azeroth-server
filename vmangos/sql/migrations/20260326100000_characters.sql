DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260326100000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260326100000');
-- Add gear_tier column to guild_bot_roster
ALTER TABLE `guild_bot_roster`
  ADD COLUMN IF NOT EXISTS `gear_tier` ENUM('fresh_60','dungeon_geared','pre_raid_bis')
  NOT NULL DEFAULT 'dungeon_geared' AFTER `gear_score`;
-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
