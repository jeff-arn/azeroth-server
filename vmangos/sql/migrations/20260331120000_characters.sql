DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331120000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331120000');
-- Add your query below.

-- Add personality column to guild_bot_roster for chat style variation.
ALTER TABLE `guild_bot_roster`
  ADD COLUMN `personality` ENUM('enthusiastic','chill','sarcastic','quiet')
  NOT NULL DEFAULT 'chill' AFTER `gear_tier`;

-- Seed personalities across the 20 guild bots (5 enthusiastic, 6 chill, 5 sarcastic, 4 quiet).
-- GUIDs 200-219 are guild bots per CLAUDE.md.
UPDATE `guild_bot_roster` SET `personality` = 'enthusiastic' WHERE `guid` IN (200, 205, 210, 215, 218);
UPDATE `guild_bot_roster` SET `personality` = 'chill'        WHERE `guid` IN (201, 206, 211, 213, 216, 219);
UPDATE `guild_bot_roster` SET `personality` = 'sarcastic'    WHERE `guid` IN (202, 207, 212, 214, 217);
UPDATE `guild_bot_roster` SET `personality` = 'quiet'        WHERE `guid` IN (203, 204, 208, 209);

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
