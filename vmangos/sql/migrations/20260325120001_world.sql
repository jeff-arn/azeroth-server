DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260325120001');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260325120001');
-- Add your query below.


-- Wipe old data and repopulate with corrected prices, farmability weights, and categories.
-- Prices are in copper. 10000 = 1g. Prices are PER STACK (bid/buyout for the listed quantity).
-- Weight controls selection frequency: higher = appears more often on the AH.
DELETE FROM `auctionhousebot`;

-- ============================================================================
-- TRADE GOODS: Herbs (leveling)
-- ============================================================================
-- Peacebloom (herb, lvl 1)
INSERT INTO `auctionhousebot` VALUES (2447, 5, 20, 1500, 2500, 40, 'leveling', 15);
-- Silverleaf (herb, lvl 1)
INSERT INTO `auctionhousebot` VALUES (765, 5, 20, 1500, 2500, 40, 'leveling', 15);
-- Earthroot (herb, lvl 1)
INSERT INTO `auctionhousebot` VALUES (2449, 5, 20, 2000, 3000, 35, 'leveling', 15);
-- Mageroyal (herb, lvl 8)
INSERT INTO `auctionhousebot` VALUES (785, 5, 20, 3000, 5000, 30, 'leveling', 15);
-- Briarthorn (herb, lvl 10)
INSERT INTO `auctionhousebot` VALUES (2450, 5, 20, 4000, 6000, 28, 'leveling', 15);
-- Swiftthistle (herb, lvl 10)
INSERT INTO `auctionhousebot` VALUES (2452, 5, 20, 5000, 8000, 20, 'leveling', 15);
-- Bruiseweed (herb, lvl 15)
INSERT INTO `auctionhousebot` VALUES (2453, 5, 20, 4000, 6000, 25, 'leveling', 15);
-- Stranglekelp (herb, lvl 20)
INSERT INTO `auctionhousebot` VALUES (3820, 5, 20, 8000, 12000, 20, 'leveling', 15);
-- Wild Steelbloom (herb, lvl 20)
INSERT INTO `auctionhousebot` VALUES (3355, 5, 20, 6000, 10000, 20, 'leveling', 15);
-- Grave Moss (herb, lvl 20)
INSERT INTO `auctionhousebot` VALUES (3369, 5, 20, 6000, 10000, 15, 'leveling', 15);
-- Kingsblood (herb, lvl 25)
INSERT INTO `auctionhousebot` VALUES (3356, 5, 20, 8000, 12000, 22, 'leveling', 15);
-- Liferoot (herb, lvl 25)
INSERT INTO `auctionhousebot` VALUES (3357, 5, 20, 8000, 12000, 20, 'leveling', 15);
-- Fadeleaf (herb, lvl 30)
INSERT INTO `auctionhousebot` VALUES (3818, 5, 20, 10000, 15000, 15, 'leveling', 15);
-- Goldthorn (herb, lvl 30)
INSERT INTO `auctionhousebot` VALUES (3821, 5, 20, 15000, 25000, 15, 'leveling', 15);
-- Khadgar's Whisker (herb, lvl 30)
INSERT INTO `auctionhousebot` VALUES (3358, 5, 20, 12000, 18000, 15, 'leveling', 15);
-- Wintersbite (herb, lvl 30)
INSERT INTO `auctionhousebot` VALUES (3819, 5, 20, 10000, 15000, 10, 'leveling', 15);

-- ============================================================================
-- TRADE GOODS: Herbs (mid-level)
-- ============================================================================
-- Firebloom (herb, lvl 40)
INSERT INTO `auctionhousebot` VALUES (4625, 5, 20, 15000, 25000, 18, 'trade_goods', 15);
-- Purple Lotus (herb, lvl 42)
INSERT INTO `auctionhousebot` VALUES (8831, 5, 20, 15000, 25000, 16, 'trade_goods', 15);
-- Arthas' Tears (herb, lvl 42)
INSERT INTO `auctionhousebot` VALUES (8836, 5, 20, 20000, 30000, 14, 'trade_goods', 15);
-- Sungrass (herb, lvl 43)
INSERT INTO `auctionhousebot` VALUES (8838, 5, 20, 20000, 30000, 16, 'trade_goods', 15);
-- Blindweed (herb, lvl 40)
INSERT INTO `auctionhousebot` VALUES (8839, 5, 20, 15000, 25000, 14, 'trade_goods', 15);
-- Ghost Mushroom (herb, lvl 42)
INSERT INTO `auctionhousebot` VALUES (8845, 5, 20, 30000, 50000, 8, 'trade_goods', 15);
-- Gromsblood (herb, lvl 45)
INSERT INTO `auctionhousebot` VALUES (8846, 5, 20, 40000, 60000, 10, 'trade_goods', 15);
-- Wildvine (rare herb drop)
INSERT INTO `auctionhousebot` VALUES (8153, 1, 5, 15000, 25000, 6, 'trade_goods', 20);

-- ============================================================================
-- TRADE GOODS: Herbs (endgame)
-- ============================================================================
-- Dreamfoil (herb, lvl 50)
INSERT INTO `auctionhousebot` VALUES (13463, 5, 20, 50000, 80000, 20, 'trade_goods', 15);
-- Golden Sansam (herb, lvl 50)
INSERT INTO `auctionhousebot` VALUES (13464, 5, 20, 30000, 50000, 18, 'trade_goods', 15);
-- Mountain Silversage (herb, lvl 50)
INSERT INTO `auctionhousebot` VALUES (13465, 5, 20, 80000, 120000, 14, 'trade_goods', 15);
-- Plaguebloom (herb, lvl 50)
INSERT INTO `auctionhousebot` VALUES (13466, 5, 20, 40000, 60000, 16, 'trade_goods', 15);
-- Icecap (herb, lvl 55)
INSERT INTO `auctionhousebot` VALUES (13467, 5, 20, 40000, 60000, 12, 'trade_goods', 15);
-- Black Lotus (herb, ultra-rare)
INSERT INTO `auctionhousebot` VALUES (13468, 1, 1, 300000, 450000, 1, 'trade_goods', 25);

-- ============================================================================
-- TRADE GOODS: Ore (leveling)
-- ============================================================================
-- Copper Ore
INSERT INTO `auctionhousebot` VALUES (2770, 5, 20, 2000, 3000, 40, 'leveling', 15);
-- Tin Ore
INSERT INTO `auctionhousebot` VALUES (2771, 5, 20, 4000, 6000, 30, 'leveling', 15);
-- Silver Ore
INSERT INTO `auctionhousebot` VALUES (2775, 1, 3, 5000, 8000, 10, 'leveling', 20);
-- Iron Ore
INSERT INTO `auctionhousebot` VALUES (2772, 5, 20, 8000, 12000, 25, 'leveling', 15);
-- Gold Ore
INSERT INTO `auctionhousebot` VALUES (2776, 1, 3, 8000, 12000, 8, 'leveling', 20);
-- Mithril Ore
INSERT INTO `auctionhousebot` VALUES (3858, 5, 20, 15000, 25000, 20, 'trade_goods', 15);

-- ============================================================================
-- TRADE GOODS: Ore (endgame)
-- ============================================================================
-- Thorium Ore
INSERT INTO `auctionhousebot` VALUES (10620, 5, 20, 20000, 35000, 22, 'trade_goods', 15);
-- Truesilver Ore
INSERT INTO `auctionhousebot` VALUES (7911, 1, 5, 10000, 18000, 8, 'trade_goods', 20);
-- Dark Iron Ore
INSERT INTO `auctionhousebot` VALUES (11370, 1, 10, 15000, 25000, 4, 'trade_goods', 20);
-- Arcane Crystal
INSERT INTO `auctionhousebot` VALUES (12363, 1, 1, 150000, 250000, 2, 'crafting', 25);

-- ============================================================================
-- TRADE GOODS: Stone
-- ============================================================================
-- Rough Stone
INSERT INTO `auctionhousebot` VALUES (2835, 10, 20, 500, 1000, 35, 'leveling', 10);
-- Coarse Stone
INSERT INTO `auctionhousebot` VALUES (2836, 10, 20, 1000, 2000, 30, 'leveling', 10);
-- Heavy Stone
INSERT INTO `auctionhousebot` VALUES (2838, 10, 20, 2000, 3500, 25, 'leveling', 10);
-- Solid Stone
INSERT INTO `auctionhousebot` VALUES (7912, 10, 20, 3000, 5000, 20, 'leveling', 10);
-- Dense Stone
INSERT INTO `auctionhousebot` VALUES (12365, 10, 20, 5000, 8000, 18, 'trade_goods', 10);

-- ============================================================================
-- TRADE GOODS: Leather (leveling)
-- ============================================================================
-- Light Leather
INSERT INTO `auctionhousebot` VALUES (2318, 5, 20, 2000, 3000, 35, 'leveling', 15);
-- Medium Leather
INSERT INTO `auctionhousebot` VALUES (2319, 5, 20, 4000, 6000, 28, 'leveling', 15);
-- Heavy Leather
INSERT INTO `auctionhousebot` VALUES (4234, 5, 20, 6000, 10000, 22, 'leveling', 15);
-- Thick Leather
INSERT INTO `auctionhousebot` VALUES (4304, 5, 20, 8000, 12000, 20, 'trade_goods', 15);
-- Rugged Leather
INSERT INTO `auctionhousebot` VALUES (8170, 5, 20, 10000, 18000, 18, 'trade_goods', 15);
-- Rugged Hide
INSERT INTO `auctionhousebot` VALUES (8171, 1, 3, 10000, 18000, 6, 'trade_goods', 20);

-- ============================================================================
-- TRADE GOODS: Cloth
-- ============================================================================
-- Linen Cloth
INSERT INTO `auctionhousebot` VALUES (2589, 5, 20, 1500, 2500, 40, 'leveling', 10);
-- Wool Cloth
INSERT INTO `auctionhousebot` VALUES (2592, 5, 20, 3000, 5000, 30, 'leveling', 10);
-- Silk Cloth
INSERT INTO `auctionhousebot` VALUES (4306, 5, 20, 5000, 8000, 25, 'leveling', 10);
-- Mageweave Cloth
INSERT INTO `auctionhousebot` VALUES (4338, 5, 20, 10000, 15000, 22, 'trade_goods', 10);
-- Runecloth
INSERT INTO `auctionhousebot` VALUES (14047, 5, 20, 10000, 18000, 30, 'trade_goods', 10);
-- Felcloth
INSERT INTO `auctionhousebot` VALUES (14256, 1, 3, 15000, 30000, 6, 'crafting', 20);
-- Mooncloth
INSERT INTO `auctionhousebot` VALUES (14342, 1, 1, 80000, 120000, 2, 'crafting', 20);

-- ============================================================================
-- CRAFTING: Bars and refined materials
-- ============================================================================
-- Copper Bar
INSERT INTO `auctionhousebot` VALUES (2840, 5, 20, 2000, 3500, 30, 'leveling', 10);
-- Tin Bar
INSERT INTO `auctionhousebot` VALUES (3576, 5, 20, 4000, 6500, 22, 'leveling', 10);
-- Bronze Bar
INSERT INTO `auctionhousebot` VALUES (2841, 5, 20, 5000, 8000, 20, 'leveling', 10);
-- Iron Bar
INSERT INTO `auctionhousebot` VALUES (3575, 5, 20, 8000, 13000, 20, 'leveling', 10);
-- Steel Bar
INSERT INTO `auctionhousebot` VALUES (3859, 5, 20, 12000, 18000, 12, 'leveling', 10);
-- Mithril Bar
INSERT INTO `auctionhousebot` VALUES (3860, 5, 20, 15000, 25000, 16, 'trade_goods', 10);
-- Thorium Bar
INSERT INTO `auctionhousebot` VALUES (12359, 5, 20, 25000, 40000, 18, 'crafting', 10);
-- Arcanite Bar
INSERT INTO `auctionhousebot` VALUES (12360, 1, 1, 200000, 300000, 2, 'crafting', 25);

-- ============================================================================
-- CRAFTING: Essences and rare materials
-- ============================================================================
-- Essence of Fire
INSERT INTO `auctionhousebot` VALUES (7078, 1, 3, 15000, 25000, 5, 'crafting', 20);
-- Essence of Water
INSERT INTO `auctionhousebot` VALUES (7080, 1, 3, 20000, 35000, 4, 'crafting', 20);
-- Essence of Air
INSERT INTO `auctionhousebot` VALUES (7082, 1, 3, 25000, 40000, 3, 'crafting', 20);
-- Essence of Earth
INSERT INTO `auctionhousebot` VALUES (7076, 1, 3, 10000, 18000, 5, 'crafting', 20);
-- Essence of Undeath
INSERT INTO `auctionhousebot` VALUES (12808, 1, 3, 12000, 20000, 5, 'crafting', 20);
-- Heart of Fire
INSERT INTO `auctionhousebot` VALUES (7077, 1, 3, 8000, 15000, 6, 'crafting', 20);
-- Globe of Water
INSERT INTO `auctionhousebot` VALUES (7079, 1, 3, 10000, 18000, 5, 'crafting', 20);
-- Breath of Wind
INSERT INTO `auctionhousebot` VALUES (7081, 1, 3, 12000, 20000, 4, 'crafting', 20);
-- Core of Earth
INSERT INTO `auctionhousebot` VALUES (7075, 1, 3, 6000, 10000, 6, 'crafting', 20);
-- Living Essence
INSERT INTO `auctionhousebot` VALUES (12803, 1, 3, 15000, 25000, 4, 'crafting', 20);
-- Righteous Orb
INSERT INTO `auctionhousebot` VALUES (12811, 1, 1, 30000, 50000, 3, 'crafting', 25);
-- Cured Rugged Hide
INSERT INTO `auctionhousebot` VALUES (15407, 1, 3, 20000, 35000, 4, 'crafting', 20);
-- Refined Deeprock Salt
INSERT INTO `auctionhousebot` VALUES (18567, 1, 1, 15000, 25000, 3, 'crafting', 20);
-- Large Brilliant Shard
INSERT INTO `auctionhousebot` VALUES (14344, 1, 3, 40000, 60000, 4, 'crafting', 20);
-- Nexus Crystal
INSERT INTO `auctionhousebot` VALUES (20725, 1, 1, 50000, 80000, 2, 'crafting', 25);

-- ============================================================================
-- CRAFTING: Enchanting materials (leveling)
-- ============================================================================
-- Strange Dust
INSERT INTO `auctionhousebot` VALUES (10940, 5, 20, 3000, 5000, 25, 'leveling', 10);
-- Soul Dust
INSERT INTO `auctionhousebot` VALUES (11083, 5, 20, 4000, 7000, 22, 'leveling', 10);
-- Vision Dust
INSERT INTO `auctionhousebot` VALUES (11137, 5, 20, 8000, 12000, 18, 'leveling', 10);
-- Dream Dust
INSERT INTO `auctionhousebot` VALUES (11176, 5, 20, 10000, 15000, 18, 'trade_goods', 10);
-- Illusion Dust
INSERT INTO `auctionhousebot` VALUES (16204, 5, 20, 20000, 30000, 12, 'trade_goods', 10);
-- Lesser Magic Essence
INSERT INTO `auctionhousebot` VALUES (10938, 3, 10, 2000, 3500, 20, 'leveling', 10);
-- Lesser Astral Essence
INSERT INTO `auctionhousebot` VALUES (10998, 3, 10, 4000, 6000, 16, 'leveling', 10);
-- Lesser Mystic Essence
INSERT INTO `auctionhousebot` VALUES (11134, 3, 10, 5000, 8000, 14, 'leveling', 10);
-- Lesser Nether Essence
INSERT INTO `auctionhousebot` VALUES (11174, 3, 10, 10000, 15000, 10, 'trade_goods', 10);
-- Lesser Eternal Essence
INSERT INTO `auctionhousebot` VALUES (16202, 3, 10, 15000, 25000, 8, 'trade_goods', 10);
-- Small Glimmering Shard
INSERT INTO `auctionhousebot` VALUES (10978, 1, 3, 1500, 3000, 12, 'leveling', 15);
-- Large Glimmering Shard
INSERT INTO `auctionhousebot` VALUES (11084, 1, 3, 2000, 4000, 10, 'leveling', 15);
-- Small Glowing Shard
INSERT INTO `auctionhousebot` VALUES (11138, 1, 3, 3000, 5000, 10, 'leveling', 15);
-- Large Glowing Shard
INSERT INTO `auctionhousebot` VALUES (11139, 1, 3, 5000, 8000, 8, 'leveling', 15);
-- Small Radiant Shard
INSERT INTO `auctionhousebot` VALUES (11177, 1, 3, 8000, 15000, 6, 'trade_goods', 15);
-- Large Radiant Shard
INSERT INTO `auctionhousebot` VALUES (11178, 1, 3, 15000, 25000, 5, 'trade_goods', 15);
-- Small Brilliant Shard
INSERT INTO `auctionhousebot` VALUES (14343, 1, 3, 10000, 18000, 5, 'trade_goods', 15);

-- ============================================================================
-- CRAFTING: Enchanting rods
-- ============================================================================
-- Silver Rod
INSERT INTO `auctionhousebot` VALUES (6338, 1, 1, 5000, 10000, 4, 'leveling', 10);
-- Golden Rod
INSERT INTO `auctionhousebot` VALUES (11128, 1, 1, 15000, 25000, 3, 'leveling', 10);
-- Truesilver Rod
INSERT INTO `auctionhousebot` VALUES (11144, 1, 1, 30000, 50000, 3, 'trade_goods', 10);
-- Arcanite Rod
INSERT INTO `auctionhousebot` VALUES (16206, 1, 1, 250000, 400000, 1, 'crafting', 15);

-- ============================================================================
-- CONSUMABLES: Healing and Mana Potions
-- ============================================================================
-- Minor Healing Potion
INSERT INTO `auctionhousebot` VALUES (118, 1, 5, 100, 200, 20, 'leveling', 10);
-- Lesser Healing Potion
INSERT INTO `auctionhousebot` VALUES (858, 1, 5, 200, 400, 18, 'leveling', 10);
-- Healing Potion
INSERT INTO `auctionhousebot` VALUES (929, 1, 5, 500, 1000, 16, 'leveling', 10);
-- Greater Healing Potion
INSERT INTO `auctionhousebot` VALUES (1710, 1, 5, 1500, 3000, 14, 'consumables', 10);
-- Superior Healing Potion
INSERT INTO `auctionhousebot` VALUES (3928, 1, 5, 3000, 5000, 12, 'consumables', 10);
-- Major Healing Potion
INSERT INTO `auctionhousebot` VALUES (13446, 1, 5, 5000, 8000, 15, 'consumables', 10);
-- Minor Mana Potion
INSERT INTO `auctionhousebot` VALUES (2455, 1, 5, 100, 200, 18, 'leveling', 10);
-- Lesser Mana Potion
INSERT INTO `auctionhousebot` VALUES (3385, 1, 5, 300, 600, 16, 'leveling', 10);
-- Mana Potion
INSERT INTO `auctionhousebot` VALUES (3827, 1, 5, 800, 1500, 14, 'leveling', 10);
-- Greater Mana Potion
INSERT INTO `auctionhousebot` VALUES (6149, 1, 5, 2000, 3500, 12, 'consumables', 10);
-- Superior Mana Potion
INSERT INTO `auctionhousebot` VALUES (13443, 1, 5, 4000, 6000, 12, 'consumables', 10);
-- Major Mana Potion
INSERT INTO `auctionhousebot` VALUES (13444, 1, 5, 8000, 15000, 10, 'consumables', 10);

-- ============================================================================
-- CONSUMABLES: Elixirs
-- ============================================================================
-- Elixir of the Mongoose
INSERT INTO `auctionhousebot` VALUES (13452, 1, 5, 20000, 35000, 8, 'consumables', 15);
-- Elixir of Greater Agility
INSERT INTO `auctionhousebot` VALUES (9187, 1, 5, 8000, 15000, 10, 'consumables', 15);
-- Elixir of the Giants
INSERT INTO `auctionhousebot` VALUES (9206, 1, 5, 8000, 15000, 10, 'consumables', 15);
-- Greater Arcane Elixir
INSERT INTO `auctionhousebot` VALUES (13454, 1, 5, 12000, 20000, 8, 'consumables', 15);
-- Elixir of Shadow Power
INSERT INTO `auctionhousebot` VALUES (9264, 1, 5, 15000, 25000, 6, 'consumables', 15);
-- Elixir of Greater Firepower
INSERT INTO `auctionhousebot` VALUES (21546, 1, 5, 12000, 20000, 6, 'consumables', 15);
-- Elixir of Brute Force
INSERT INTO `auctionhousebot` VALUES (13453, 1, 5, 10000, 18000, 8, 'consumables', 15);

-- ============================================================================
-- CONSUMABLES: Protection Potions
-- ============================================================================
-- Greater Fire Protection Potion
INSERT INTO `auctionhousebot` VALUES (13457, 1, 5, 20000, 40000, 6, 'consumables', 20);
-- Greater Nature Protection Potion
INSERT INTO `auctionhousebot` VALUES (13458, 1, 5, 20000, 40000, 6, 'consumables', 20);
-- Greater Shadow Protection Potion
INSERT INTO `auctionhousebot` VALUES (13459, 1, 5, 15000, 30000, 6, 'consumables', 20);
-- Greater Frost Protection Potion
INSERT INTO `auctionhousebot` VALUES (13456, 1, 5, 15000, 30000, 5, 'consumables', 20);
-- Greater Arcane Protection Potion
INSERT INTO `auctionhousebot` VALUES (13461, 1, 5, 15000, 30000, 5, 'consumables', 20);
-- Free Action Potion
INSERT INTO `auctionhousebot` VALUES (5634, 1, 5, 10000, 18000, 6, 'consumables', 15);
-- Limited Invulnerability Potion
INSERT INTO `auctionhousebot` VALUES (3387, 1, 5, 20000, 35000, 4, 'consumables', 15);

-- ============================================================================
-- CONSUMABLES: Flasks
-- ============================================================================
-- Flask of the Titans
INSERT INTO `auctionhousebot` VALUES (13510, 1, 1, 350000, 500000, 1, 'consumables', 20);
-- Flask of Supreme Power
INSERT INTO `auctionhousebot` VALUES (13512, 1, 1, 350000, 500000, 1, 'consumables', 20);
-- Flask of Distilled Wisdom
INSERT INTO `auctionhousebot` VALUES (13511, 1, 1, 300000, 450000, 1, 'consumables', 20);
-- Flask of Chromatic Resistance
INSERT INTO `auctionhousebot` VALUES (13513, 1, 1, 300000, 450000, 1, 'consumables', 20);

-- ============================================================================
-- CONSUMABLES: Food and Cooking
-- ============================================================================
-- Nightfin Soup
INSERT INTO `auctionhousebot` VALUES (13931, 1, 5, 3000, 6000, 10, 'consumables', 15);
-- Grilled Squid
INSERT INTO `auctionhousebot` VALUES (13928, 1, 5, 3000, 6000, 10, 'consumables', 15);
-- Runn Tum Tuber Surprise
INSERT INTO `auctionhousebot` VALUES (18254, 1, 5, 5000, 10000, 5, 'consumables', 15);
-- Smoked Desert Dumplings
INSERT INTO `auctionhousebot` VALUES (20452, 1, 5, 3000, 6000, 8, 'consumables', 15);
-- Tender Wolf Steak
INSERT INTO `auctionhousebot` VALUES (18045, 1, 5, 2000, 4000, 10, 'consumables', 15);

-- ============================================================================
-- CONSUMABLES: Cooking fish and ingredients
-- ============================================================================
-- Nightfin Snapper
INSERT INTO `auctionhousebot` VALUES (13759, 1, 10, 3000, 5000, 8, 'trade_goods', 15);
-- Winter Squid
INSERT INTO `auctionhousebot` VALUES (13755, 1, 10, 3000, 5000, 8, 'trade_goods', 15);
-- Stonescale Eel
INSERT INTO `auctionhousebot` VALUES (13754, 1, 10, 5000, 8000, 6, 'trade_goods', 15);
-- Raw Whitescale Salmon
INSERT INTO `auctionhousebot` VALUES (13889, 1, 10, 2000, 4000, 8, 'trade_goods', 15);

-- ============================================================================
-- CONSUMABLES: Bandages and utility
-- ============================================================================
-- Heavy Runecloth Bandage
INSERT INTO `auctionhousebot` VALUES (14530, 5, 20, 5000, 8000, 10, 'consumables', 10);
-- Goblin Rocket Fuel
INSERT INTO `auctionhousebot` VALUES (9061, 1, 5, 5000, 10000, 4, 'consumables', 15);

-- ============================================================================
-- CONSUMABLES: Sharpening and Weapon Oils
-- ============================================================================
-- Elemental Sharpening Stone
INSERT INTO `auctionhousebot` VALUES (18262, 1, 5, 15000, 25000, 4, 'consumables', 15);
-- Dense Sharpening Stone
INSERT INTO `auctionhousebot` VALUES (12404, 1, 5, 3000, 5000, 8, 'consumables', 10);
-- Brilliant Wizard Oil
INSERT INTO `auctionhousebot` VALUES (20749, 1, 5, 10000, 18000, 4, 'consumables', 15);
-- Brilliant Mana Oil
INSERT INTO `auctionhousebot` VALUES (20748, 1, 5, 10000, 18000, 4, 'consumables', 15);

-- ============================================================================
-- RECIPES: Common (vendor-available or easy drops)
-- ============================================================================
-- Plans: Runed Copper Breastplate
INSERT INTO `auctionhousebot` VALUES (2881, 1, 1, 2000, 5000, 2, 'recipes', 15);
-- Recipe: Elixir of Minor Agility
INSERT INTO `auctionhousebot` VALUES (2553, 1, 1, 3000, 8000, 2, 'recipes', 15);
-- Recipe: Free Action Potion
INSERT INTO `auctionhousebot` VALUES (5640, 1, 1, 20000, 40000, 1, 'recipes', 20);
-- Pattern: Runecloth Bag
INSERT INTO `auctionhousebot` VALUES (14468, 1, 1, 15000, 25000, 2, 'recipes', 15);

-- ============================================================================
-- RECIPES: Uncommon (rep vendors, uncommon drops)
-- ============================================================================
-- Formula: Enchant Bracer - Superior Stamina
INSERT INTO `auctionhousebot` VALUES (16218, 1, 1, 50000, 100000, 1, 'recipes', 20);
-- Recipe: Greater Fire Protection Potion
INSERT INTO `auctionhousebot` VALUES (13485, 1, 1, 40000, 80000, 1, 'recipes', 20);
-- Recipe: Greater Nature Protection Potion
INSERT INTO `auctionhousebot` VALUES (13486, 1, 1, 40000, 80000, 1, 'recipes', 20);

-- ============================================================================
-- RECIPES: Rare (world drops, valuable)
-- ============================================================================
-- Formula: Enchant Weapon - Crusader
INSERT INTO `auctionhousebot` VALUES (16252, 1, 1, 1500000, 2500000, 0.3, 'recipes', 25);
-- Formula: Enchant Weapon - Spell Power
INSERT INTO `auctionhousebot` VALUES (16255, 1, 1, 3000000, 5000000, 0.1, 'recipes', 25);
-- Formula: Enchant Weapon - Healing Power
INSERT INTO `auctionhousebot` VALUES (16253, 1, 1, 2500000, 4000000, 0.1, 'recipes', 25);
-- Plans: Lionheart Helm
INSERT INTO `auctionhousebot` VALUES (12717, 1, 1, 4000000, 6000000, 0.1, 'recipes', 25);
-- Pattern: Hide of the Wild
INSERT INTO `auctionhousebot` VALUES (15769, 1, 1, 1000000, 2000000, 0.2, 'recipes', 25);
-- Formula: Enchant Weapon - Agility
INSERT INTO `auctionhousebot` VALUES (16254, 1, 1, 1000000, 1800000, 0.2, 'recipes', 25);


-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
