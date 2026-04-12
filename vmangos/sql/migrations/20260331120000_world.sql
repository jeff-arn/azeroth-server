DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331120000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331120000');
-- Add your query below.


-- ============================================================================
-- EQUIPMENT: Bags
-- Bags are essential for all players but were missing from the AH bot catalog.
-- Prices reflect vanilla-era AH market values. Weight scaled by demand/level.
-- ============================================================================
-- Linen Bag (6 slot, tailoring 45)
INSERT INTO `auctionhousebot` VALUES (4238, 1, 3, 500, 1000, 15, 'equipment', 10);
-- Woolen Bag (8 slot, tailoring 80)
INSERT INTO `auctionhousebot` VALUES (4240, 1, 3, 1500, 3000, 12, 'equipment', 10);
-- Small Silk Pack (10 slot, tailoring 150)
INSERT INTO `auctionhousebot` VALUES (4245, 1, 2, 5000, 10000, 10, 'equipment', 10);
-- Mageweave Bag (12 slot, tailoring 225)
INSERT INTO `auctionhousebot` VALUES (10050, 1, 2, 15000, 25000, 10, 'equipment', 10);
-- Runecloth Bag (14 slot, tailoring 260)
INSERT INTO `auctionhousebot` VALUES (14046, 1, 2, 25000, 45000, 8, 'equipment', 15);
-- Traveler's Backpack (16 slot, world drop)
INSERT INTO `auctionhousebot` VALUES (4500, 1, 1, 50000, 100000, 2, 'equipment', 20);
-- Mooncloth Bag (16 slot, tailoring 300, mooncloth CD)
INSERT INTO `auctionhousebot` VALUES (14155, 1, 1, 80000, 150000, 2, 'equipment', 20);
-- Bottomless Bag (18 slot, rare craft)
INSERT INTO `auctionhousebot` VALUES (14156, 1, 1, 1500000, 2500000, 0.3, 'equipment', 25);

-- Specialty bags
-- Soul Pouch (20 slot soul shard bag, tailoring 260)
INSERT INTO `auctionhousebot` VALUES (21340, 1, 1, 20000, 40000, 2, 'equipment', 15);
-- Enchanted Mageweave Pouch (16 slot enchanting bag)
INSERT INTO `auctionhousebot` VALUES (22246, 1, 1, 30000, 60000, 2, 'equipment', 15);
-- Big Bag of Enchantment (20 slot enchanting bag)
INSERT INTO `auctionhousebot` VALUES (22249, 1, 1, 80000, 150000, 1, 'equipment', 20);
-- Herb Pouch (12 slot herb bag)
INSERT INTO `auctionhousebot` VALUES (22250, 1, 1, 15000, 30000, 2, 'equipment', 15);
-- Satchel of Cenarius (20 slot herb bag)
INSERT INTO `auctionhousebot` VALUES (22252, 1, 1, 50000, 100000, 1, 'equipment', 20);


-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
