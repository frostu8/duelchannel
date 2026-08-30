-- Recreate this bullshit again
PRAGMA defer_foreign_keys = ON;

CREATE TABLE battle_new (
    id INTEGER PRIMARY KEY,
    -- The server the battle took place on.
    server_id INTEGER REFERENCES server(id),
    -- The unique identiifer of the battle.
    uuid CHAR(36) NOT NULL UNIQUE,
    -- The name of the level of the battle.
    level_name VARCHAR(255) NOT NULL,
    -- The internal identifier of the level (the map lumpname).
    level_id VARCHAR(255) NOT NULL,
    -- Level status.
    status INTEGER NOT NULL DEFAULT 0,
    -- The final overtimecheckpoints of the battle.
    margin_score INTEGER NOT NULL DEFAULT 0,
    -- The replay hash and filename of the replay.
    replay_hash CHAR(64),
    replay_filename VARCHAR(256),
    -- When the battle concluded.
    concluded_at TIMESTAMP,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- Backfill using old map names
-- (these should stay consistent in the same game version)
INSERT INTO battle_new
    (
        id, server_id, uuid, level_name, status, margin_score,
        replay_hash, replay_filename, concluded_at, inserted_at, updated_at,
        level_id
    )
SELECT
    id, server_id, uuid, level_name, status, margin_score,
    replay_hash, replay_filename, concluded_at, inserted_at, updated_at,
    (CASE level_name
        WHEN '765 Stadium Zone' THEN 'RR_765STADIUM'
        WHEN 'Abyss Garden Zone' THEN 'RR_ABYSSGARDEN'
        WHEN 'Advent Angel Zone' THEN 'RR_ADVENTANGEL'
        WHEN 'Aerial Highlands Zone' THEN 'RR_AERIALHIGHLANDS'
        WHEN 'Angel Arrow Classic' THEN 'RR_ANGELARROWCLASSIC'
        WHEN 'Angel Island Zone' THEN 'RR_ANGELISLAND'
        WHEN 'Aqua Tunnel' THEN 'RR_AQUATUNNEL'
        WHEN 'Aqueduct Crystal Zone' THEN 'RR_AQUEDUCTCRYSTAL'
        WHEN 'Aurora Atoll Zone' THEN 'RR_AURORAATOLL'
        WHEN 'Autumn Ring Zone' THEN 'RR_AUTUMNRING'
        WHEN 'Avant Garden Zone' THEN 'RR_AVANTGARDEN'
        WHEN 'Azure Axiom Zone' THEN 'RR_AZUREAXIOM'
        WHEN 'Azure City Zone' THEN 'RR_AZURECITY'
        WHEN 'Azure Lake Zone' THEN 'RR_AZURELAKE'
        WHEN 'Balloon Park Zone' THEN 'RR_BALLOONPARK'
        WHEN 'Barren Badlands Zone' THEN 'RR_BARRENBADLANDS'
        WHEN 'Bigtime Breakdown Zone' THEN 'RR_BIGTIMEBREAKDOWN'
        WHEN 'Blizzard Peaks Classic' THEN 'RR_BLIZZARDPEAKSCLASSIC'
        WHEN 'Blizzard Peaks Zone' THEN 'RR_BLIZZARDPEAKS'
        WHEN 'Blue Mountain Classic' THEN 'RR_BLUEMOUNTAINCLASSIC'
        WHEN 'Blue Mountain Zone 1' THEN 'RR_BLUEMOUNTAIN1'
        WHEN 'Blue Mountain Zone 2' THEN 'RR_BLUEMOUNTAIN2'
        WHEN 'Bronze Lake Zone' THEN 'RR_BRONZELAKE'
        WHEN 'Cadillac Canyon Classic' THEN 'RR_CADILLACCANYONCLASSIC'
        WHEN 'Cadillac Cascade Zone' THEN 'RR_CADILLACCASCADE'
        WHEN 'Carnival Night Zone' THEN 'RR_CARNIVALNIGHT'
        WHEN 'Chaos Chute Zone' THEN 'RR_CHAOSTUBE'
        WHEN 'Chemical Facility Zone' THEN 'RR_CHEMICALFACILITY'
        WHEN 'Chrome Gadget Zone' THEN 'RR_CHROMEGADGET'
        WHEN 'City Escape' THEN 'RR_CITYESCAPE'
        WHEN 'Cloudtop Dimension 4' THEN 'RR_CLOUDTOPDIMENSION'
        WHEN 'Coastal Temple Zone' THEN 'RR_COASTALTEMPLE'
        WHEN 'Collision Chaos Zone' THEN 'RR_COLLISIONCHAOS'
        WHEN 'Crimson Core Zone' THEN 'RR_CRIMSONCORE'
        WHEN 'Crispy Canyon Zone' THEN 'RR_CRISPYCANYON'
        WHEN 'Cyan Belltower Zone' THEN 'RR_CYANBELLTOWER'
        WHEN 'Dark Fortress Zone' THEN 'RR_DARKFORTRESS'
        WHEN 'Darkvile Castle Zone 1' THEN 'RR_DARKVILECASTLE1'
        WHEN 'Darkvile Castle Zone 2' THEN 'RR_DARKVILECASTLE2'
        WHEN 'Daytona Speedway Zone' THEN 'RR_DAYTONASPEEDWAY'
        WHEN 'Death Egg Zone' THEN 'RR_DEATHEGG'
        WHEN 'Desert Palace Zone' THEN 'RR_DESERTPALACE'
        WHEN 'Diamond Dust Classic' THEN 'RR_DIAMONDDUSTCLASSIC'
        WHEN 'Diamond Dust Zone' THEN 'RR_DIAMONDDUST'
        WHEN 'Dimension Disaster Zone' THEN 'RR_DIMENSIONDISASTER'
        WHEN 'Dragonspire Sewer Zone 1' THEN 'RR_DRAGONSPIRESEWER1'
        WHEN 'Dragonspire Sewer Zone 2' THEN 'RR_DRAGONSPIRESEWER2'
        WHEN 'Emerald Coast' THEN 'RR_EMERALDCOAST'
        WHEN 'Emerald Hill Zone' THEN 'RR_EMERALDHILL'
        WHEN 'Endless Mine Zone 1' THEN 'RR_ENDLESSMINE'
        WHEN 'Endless Mine Zone 2' THEN 'RR_ENDLESSMINE2'
        WHEN 'Espresso Lane Zone' THEN 'RR_ESPRESSOLANE'
        WHEN 'Fae Falls Zone' THEN 'RR_FAEFALLS'
        WHEN 'Final Fall Zone' THEN 'RR_FINALFALL'
        WHEN 'Frozen Production Zone' THEN 'RR_FROZENPRODUCTION'
        WHEN 'Gigapolis Zone' THEN 'RR_GIGAPOLIS'
        WHEN 'Gravtech Dimension 5' THEN 'RR_GRAVTECHDIMENSION'
        WHEN 'Green Hills Zone' THEN 'RR_GREENHILLS'
        WHEN 'Green Triangle Zone' THEN 'RR_GREENTRIANGLE'
        WHEN 'Gust Planet Zone' THEN 'RR_GUSTPLANET'
        WHEN 'Hanagumi Hall Zone' THEN 'RR_HANAGUMIHALL'
        WHEN 'Hard-Boiled Stadium Zone' THEN 'RR_HARDBOILEDSTADIUM'
        WHEN 'Hardhat Havoc Zone' THEN 'RR_HARDHATHAVOC'
        WHEN 'Haunted Ship' THEN 'RR_HAUNTEDSHIP'
        WHEN 'Hidden Palace Zone' THEN 'RR_HIDDENPALACE'
        WHEN 'Hill Top Zone' THEN 'RR_HILLTOP'
        WHEN 'Hot Crater Zone' THEN 'RR_HOTCRATER'
        WHEN 'Hot Shelter' THEN 'RR_HOTSHELTER'
        WHEN 'Hydro City Zone' THEN 'RR_HYDROCITY'
        WHEN 'Ice Paradise Zone' THEN 'RR_ICEPARADISE'
        WHEN 'Isolated Island Zone' THEN 'RR_ISOLATEDISLAND'
        WHEN 'Joypolis Zone' THEN 'RR_JOYPOLIS'
        WHEN 'Kodachrome Void Zone' THEN 'RR_KODACHROMEVOID'
        WHEN 'Labyrinth Zone' THEN 'RR_LABYRINTH'
        WHEN 'Lake Margorite Zone' THEN 'RR_LAKEMARGORITE'
        WHEN 'Las Vegas' THEN 'RR_LASVEGAS'
        WHEN 'Launch Base Classic' THEN 'RR_LAUNCHBASECLASSIC'
        WHEN 'Launch Base Zone' THEN 'RR_LAUNCHBASE'
        WHEN 'Lavender Shrine Classic' THEN 'RR_LAVENDERSHRINECLASSIC'
        WHEN 'Lavender Shrine Zone' THEN 'RR_LAVENDERSHRINE'
        WHEN 'Leaf Storm Zone' THEN 'RR_LEAFSTORM'
        WHEN 'Lost Colony' THEN 'RR_LOSTCOLONY'
        WHEN 'Lucid Pass Zone' THEN 'RR_LUCIDPASS'
        WHEN 'Marble Garden Zone' THEN 'RR_MARBLEGARDEN'
        WHEN 'Mega Aqua Lake Zone' THEN 'RR_MEGAAQUALAKE'
        WHEN 'Mega Bridge Zone' THEN 'RR_MEGABRIDGE'
        WHEN 'Mega Collision Chaos Zone' THEN 'RR_MEGACOLLISIONCHAOS'
        WHEN 'Mega Flying Battery Zone' THEN 'RR_MEGAFLYINGBATTERY'
        WHEN 'Mega Green Hill Zone' THEN 'RR_MEGAGREENHILL'
        WHEN 'Mega Ice Cap Zone' THEN 'RR_MEGAICECAP'
        WHEN 'Mega Lava Reef Zone' THEN 'RR_MEGALAVAREEF'
        WHEN 'Mega Sandopolis Zone' THEN 'RR_MEGASANDOPOLIS'
        WHEN 'Mega Scrap Brain Zone' THEN 'RR_MEGASCRAPBRAIN'
        WHEN 'Mega Star Light Zone' THEN 'RR_MEGASTARLIGHT'
        WHEN 'Melty Manor Zone' THEN 'RR_MELTYMANOR'
        WHEN 'Metropolis Zone' THEN 'RR_METROPOLIS'
        WHEN 'Mirage Saloon Zone' THEN 'RR_MIRAGESALOON'
        WHEN 'Monkey Mall' THEN 'RR_MONKEYMALL'
        WHEN 'Motobug Motorway Zone' THEN 'RR_MOTOBUGMOTORWAY'
        WHEN 'Mystic Cave Zone' THEN 'RR_MYSTICCAVE'
        WHEN 'Nightfall Dimension 2' THEN 'RR_NIGHTFALLDIMENSION'
        WHEN 'Northern District Zone' THEN 'RR_NORTHERNDISTRICT'
        WHEN 'Nova Shore Zone' THEN 'RR_NOVASHORE'
        WHEN 'Obsidian Oasis Zone' THEN 'RR_OBSIDIANOASIS'
        WHEN 'Operator''s Overspace' THEN 'RR_OPERATORSOVERSPACE'
        WHEN 'Opulence Zone' THEN 'RR_OPULENCE'
        WHEN 'Palmtree Panic Zone' THEN 'RR_PALMTREEPANIC'
        WHEN 'Panic City Zone' THEN 'RR_PANICCITY'
        WHEN 'Pestilence Zone' THEN 'RR_PESTILENCE'
        WHEN 'Pico Park Zone' THEN 'RR_PICOPARK'
        WHEN 'Popcorn Workshop Zone' THEN 'RR_POPCORNWORKSHOP'
        WHEN 'Press Garden Zone' THEN 'RR_PRESSGARDEN'
        WHEN 'Quartz Quadrant Zone' THEN 'RR_QUARTZQUADRANT'
        WHEN 'Ramp Park Zone' THEN 'RR_RAMPPARK'
        WHEN 'Regal Ruin' THEN 'RR_REGALRUIN'
        WHEN 'Roasted Ruins Zone' THEN 'RR_ROASTEDRUINS'
        WHEN 'Robotnik Coaster Zone' THEN 'RR_ROBOTNIKCOASTER'
        WHEN 'Robotnik Winter Zone' THEN 'RR_ROBOTNIKWINTER'
        WHEN 'Route 1980 Zone' THEN 'RR_ROUTE1980'
        WHEN 'Rumble Ridge Zone' THEN 'RR_RUMBLERIDGE'
        WHEN 'SRB2 Frozen Night' THEN 'RR_SRB2FROZENNIGHT'
        WHEN 'Savannah Citadel' THEN 'RR_SAVANNAHCITADEL'
        WHEN 'Scarlet Gardens Zone' THEN 'RR_SCARLETGARDENS'
        WHEN 'Shuffle Square Zone' THEN 'RR_SHUFFLESQUARE'
        WHEN 'SilverCloud Island' THEN 'RR_SILVERCLOUDISLAND'
        WHEN 'Sky Babylon Zone' THEN 'RR_SKYBABYLON'
        WHEN 'Sky Sanctuary Zone' THEN 'RR_SKYSANCTUARY'
        WHEN 'Skyscraper Leaps Zone' THEN 'RR_SKYSCRAPERLEAPS'
        WHEN 'Sonic Speedway Zone' THEN 'RR_SONICSPEEDWAY'
        WHEN 'Speed Highway' THEN 'RR_SPEEDHIGHWAY'
        WHEN 'Spring Yard Zone' THEN 'RR_SPRINGYARD'
        WHEN 'Star Light Zone' THEN 'RR_STARLIGHT'
        WHEN 'Storm Rig Zone' THEN 'RR_STORMRIG'
        WHEN 'Sub-Zero Peak Zone' THEN 'RR_SUBZEROPEAK'
        WHEN 'Sundae Drive Zone' THEN 'RR_SUNDAEDRIVE'
        WHEN 'Sunset Hill Zone' THEN 'RR_SUNSETHILL'
        WHEN 'Sunsplashed Getaway Zone' THEN 'RR_SUNSPLASHEDGETAWAY'
        WHEN 'Technology Tundra' THEN 'RR_TECHNOLOGYTUNDRA'
        WHEN 'Test Track Zone' THEN 'RR_TESTTRACK'
        WHEN 'Thunder Piston Zone' THEN 'RR_THUNDERPISTON'
        WHEN 'Trap Tower' THEN 'RR_TRAPTOWER'
        WHEN 'Turquoise Hill Zone' THEN 'RR_TURQUOISEHILL'
        WHEN 'Umbrella Rushwinds Zone' THEN 'RR_UMBRELLARUSHWINDS'
        WHEN 'Vantablack Violet Zone' THEN 'RR_VANTABLACKVIOLET'
        WHEN 'Vermilion Vessel Zone' THEN 'RR_VERMILIONVESSEL'
        WHEN 'Virtual Highway Zone' THEN 'RR_VIRTUALHIGHWAY'
        WHEN 'Voiddance Dimension 3' THEN 'RR_VOIDDANCEDIMENSION'
        WHEN 'Water Palace Zone' THEN 'RR_WATERPALACE'
        WHEN 'Wavecrash Dimension 1' THEN 'RR_WAVECRASHDIMENSION'
        WHEN 'Weiss Waterway Zone' THEN 'RR_WEISSWATERWAY'
        WHEN 'Withering Chateau Zone' THEN 'RR_WITHERINGCHATEAU'
        WHEN 'Zoned City' THEN 'RR_ZONEDCITY'
        ELSE NULL
    END)
FROM battle;

DROP TABLE battle;
ALTER TABLE battle_new RENAME TO battle;

PRAGMA defer_foreign_keys = OFF;
