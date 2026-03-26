DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260326100000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260326100000');
-- Add gear_tier column to guild_bot_roster (skip if already exists)
SET @col_exists = (SELECT COUNT(*) FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'guild_bot_roster' AND COLUMN_NAME = 'gear_tier');
SET @sql = IF(@col_exists = 0, "ALTER TABLE `guild_bot_roster` ADD COLUMN `gear_tier` ENUM('fresh_60','dungeon_geared','pre_raid_bis') NOT NULL DEFAULT 'dungeon_geared' AFTER `gear_score`", 'SELECT 1');
PREPARE stmt FROM @sql;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;
-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
