DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260325120000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260325120000');
-- Add your query below.


-- Expand auctionhousebot table for weighted selection, category quotas, and stack variance.
ALTER TABLE `auctionhousebot`
  CHANGE COLUMN `stack` `min_stack` TINYINT(3) UNSIGNED NOT NULL DEFAULT 1 COMMENT 'Minimum Stack Size',
  ADD COLUMN `max_stack` TINYINT(3) UNSIGNED NOT NULL DEFAULT 1 COMMENT 'Maximum Stack Size' AFTER `min_stack`,
  ADD COLUMN `weight` FLOAT NOT NULL DEFAULT 1.0 COMMENT 'Selection weight (farmability)' AFTER `buyout`,
  ADD COLUMN `category` ENUM('trade_goods','crafting','consumables','recipes','equipment','leveling') NOT NULL DEFAULT 'trade_goods' COMMENT 'Item category for quota system' AFTER `weight`,
  ADD COLUMN `price_variance_pct` TINYINT(3) UNSIGNED NOT NULL DEFAULT 15 COMMENT 'Price jitter +/- percentage' AFTER `category`;

-- Set max_stack equal to min_stack for existing rows so behavior is unchanged until data is updated.
UPDATE `auctionhousebot` SET `max_stack` = `min_stack`;

-- State persistence table for 72-hour refresh timer.
CREATE TABLE IF NOT EXISTS `auctionhousebot_state` (
  `key_name` VARCHAR(64) NOT NULL,
  `value` BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (`key_name`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8;

INSERT IGNORE INTO `auctionhousebot_state` (`key_name`, `value`) VALUES ('last_refresh_time', 0);


-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
