-- ScrapMap procedural tile atlas baker.
--
-- Samples every tile registered in the survival tile database through the
-- engine's own terrain API and writes a top-down raster per tile UUID. The
-- output is a function of the game build alone: tiles are sampled unrotated
-- and without world-space effects, so the same atlas is valid for every world
-- and every player on this game version.
--
-- What gets sampled, and why:
--   material  the visible surface (grass / sand / dirt / rock). This is what
--             carries roads and biome shape. getColorAt alone is nearly white
--             because it is a tint over the material textures, not a colour.
--   colour    that tint, kept at lower resolution since it varies smoothly.
--   height    relief, for hillshading and water.
--
-- Progress is recorded in game storage rather than by probing the output
-- directory: sm.json.fileExists does not see files written during the same
-- session, so a directory probe re-bakes the same tiles forever.
--
-- The default budget covers the whole tile set in one pass, which costs about
-- twenty seconds on the first world load and nothing afterwards. Lower it to
-- spread the work across several loads instead.

SCRAPMAP_ATLAS_VERSION = 5

-- Samples per cell edge. Material drives the picture, so it gets one sample per
-- metre, keeping 8 m roads about 8 px wide. Tint and height vary smoothly and
-- are sampled coarser.
SCRAPMAP_ATLAS_MATERIAL_RES = 64
SCRAPMAP_ATLAS_COLOR_RES = 32
SCRAPMAP_ATLAS_HEIGHT_RES = 32
-- Ground cover: grass tufts, burnt stubble, pebbles. The material sampler only
-- reports the surface type, so without this a meadow and a forest floor are the
-- same flat green.
SCRAPMAP_ATLAS_CLUTTER_RES = 32

-- Budget is counted in cells, not tiles, because tiles range from 1x1 to 16x16
-- and a tile count would make the cost per load wildly uneven.
SCRAPMAP_ATLAS_CELL_BUDGET = 4096

local ATLAS_DIR = "$SURVIVAL_DATA/ScrapMapAtlas/"
local INDEX_PATH = "$SURVIVAL_DATA/ScrapMapAtlasIndex.json"
local STORAGE_CHANNEL = "ScrapMapAtlasBaked"
local CELL = 64

-- Surface classes, matching how the game itself collapses the eight material
-- channels in GetEffectMaterialAt.
local SURFACE_GRASS = 0
local SURFACE_SAND = 1
local SURFACE_DIRT = 2
local SURFACE_ROCK = 3
local SURFACE_THRESHOLD = 0.25

-- Changing how tiles are sampled invalidates everything baked so far.
local function bakeSignature()
	return string.format( "v%d-m%d-c%d-h%d-k%d", SCRAPMAP_ATLAS_VERSION,
		SCRAPMAP_ATLAS_MATERIAL_RES, SCRAPMAP_ATLAS_COLOR_RES, SCRAPMAP_ATLAS_HEIGHT_RES,
		SCRAPMAP_ATLAS_CLUTTER_RES )
end

local function classifySurface( m0, m1, m2, m3, m4, m5, m6, m7 )
	local grass = math.max( m4, m7 )
	local rock = math.max( m0, m2, m5 )
	local dirt = math.max( m3, m6 )
	local sand = m1
	local best, class = grass, SURFACE_GRASS
	if sand > best and sand > SURFACE_THRESHOLD then best, class = sand, SURFACE_SAND end
	if dirt > best and dirt > SURFACE_THRESHOLD then best, class = dirt, SURFACE_DIRT end
	if rock > best and rock > SURFACE_THRESHOLD then best, class = rock, SURFACE_ROCK end
	return class
end

local function encodeColor( r, g, b )
	if r < 0 then r = 0 elseif r > 1 then r = 1 end
	if g < 0 then g = 0 elseif g > 1 then g = 1 end
	if b < 0 then b = 0 elseif b > 1 then b = 1 end
	local r5 = math.floor( r * 31 + 0.5 )
	local g6 = math.floor( g * 63 + 0.5 )
	local b5 = math.floor( b * 31 + 0.5 )
	return string.format( "%04X", r5 * 2048 + g6 * 32 + b5 )
end

-- Placed objects (trees, rocks, buildings, the crashed ship) are assets, not
-- terrain, so they are invisible to the material and height samplers. Collect
-- them per tile with a local palette so the uuid is written once rather than
-- once per instance.
local ASSET_QUARTER_METRE_MAX = 4095

local function encodeAsset( paletteIndex, x, y )
	local qx = math.floor( x * 4 + 0.5 )
	local qy = math.floor( y * 4 + 0.5 )
	if qx < 0 then qx = 0 elseif qx > ASSET_QUARTER_METRE_MAX then qx = ASSET_QUARTER_METRE_MAX end
	if qy < 0 then qy = 0 elseif qy > ASSET_QUARTER_METRE_MAX then qy = ASSET_QUARTER_METRE_MAX end
	return string.format( "%03X%03X%03X", paletteIndex, qx, qy )
end

local function encodeHeight( h )
	-- Decimetres, biased so the range covers -3276.8 .. 3276.7 m.
	local v = math.floor( h * 10 + 0.5 ) + 32768
	if v < 0 then v = 0 elseif v > 65535 then v = 65535 end
	return string.format( "%04X", v )
end

-- Samples one tile into row-major rasters. Row 0 is ry = 0, the tile's south
-- edge; column 0 is rx = 0, its west edge. Rotation is deliberately not applied
-- here -- the renderer already rotates per placed cell.
local function sampleTile( uid, size )
	local materialSpan = size * SCRAPMAP_ATLAS_MATERIAL_RES
	local colorSpan = size * SCRAPMAP_ATLAS_COLOR_RES
	local heightSpan = size * SCRAPMAP_ATLAS_HEIGHT_RES
	local materialParts, colorParts, heightParts = {}, {}, {}
	local minHeight, maxHeight

	local step = CELL / SCRAPMAP_ATLAS_MATERIAL_RES
	for row = 0, materialSpan - 1 do
		local offsetY = math.floor( row / SCRAPMAP_ATLAS_MATERIAL_RES )
		local ry = ( row % SCRAPMAP_ATLAS_MATERIAL_RES + 0.5 ) * step
		for column = 0, materialSpan - 1 do
			local offsetX = math.floor( column / SCRAPMAP_ATLAS_MATERIAL_RES )
			local rx = ( column % SCRAPMAP_ATLAS_MATERIAL_RES + 0.5 ) * step
			local m0, m1, m2, m3, m4, m5, m6, m7 =
				sm.terrainTile.getMaterialAt( uid, offsetX, offsetY, 0, rx, ry )
			materialParts[#materialParts + 1] =
				string.format( "%X", classifySurface( m0, m1, m2, m3, m4, m5, m6, m7 ) )
		end
	end

	step = CELL / SCRAPMAP_ATLAS_COLOR_RES
	for row = 0, colorSpan - 1 do
		local offsetY = math.floor( row / SCRAPMAP_ATLAS_COLOR_RES )
		local ry = ( row % SCRAPMAP_ATLAS_COLOR_RES + 0.5 ) * step
		for column = 0, colorSpan - 1 do
			local offsetX = math.floor( column / SCRAPMAP_ATLAS_COLOR_RES )
			local rx = ( column % SCRAPMAP_ATLAS_COLOR_RES + 0.5 ) * step
			local r, g, b = sm.terrainTile.getColorAt( uid, offsetX, offsetY, 0, rx, ry )
			colorParts[#colorParts + 1] = encodeColor( r, g, b )
		end
	end

	-- Clutter is addressed in half-metres, not metres: the game passes
	-- CELL_SIZE * 2 - 1 as the wrap limit.
	local clutterSpan = size * SCRAPMAP_ATLAS_CLUTTER_RES
	local clutterParts = {}
	step = ( CELL * 2 ) / SCRAPMAP_ATLAS_CLUTTER_RES
	for row = 0, clutterSpan - 1 do
		local offsetY = math.floor( row / SCRAPMAP_ATLAS_CLUTTER_RES )
		local ry = ( row % SCRAPMAP_ATLAS_CLUTTER_RES + 0.5 ) * step
		for column = 0, clutterSpan - 1 do
			local offsetX = math.floor( column / SCRAPMAP_ATLAS_CLUTTER_RES )
			local rx = ( column % SCRAPMAP_ATLAS_CLUTTER_RES + 0.5 ) * step
			local ok, index = pcall( sm.terrainTile.getClutterIdxAt, uid, offsetX, offsetY, rx, ry )
			if not ok or type( index ) ~= "number" or index < 0 or index > 254 then
				index = 255
			end
			clutterParts[#clutterParts + 1] = string.format( "%02X", index )
		end
	end

	step = CELL / SCRAPMAP_ATLAS_HEIGHT_RES
	for row = 0, heightSpan - 1 do
		local offsetY = math.floor( row / SCRAPMAP_ATLAS_HEIGHT_RES )
		local ry = ( row % SCRAPMAP_ATLAS_HEIGHT_RES + 0.5 ) * step
		for column = 0, heightSpan - 1 do
			local offsetX = math.floor( column / SCRAPMAP_ATLAS_HEIGHT_RES )
			local rx = ( column % SCRAPMAP_ATLAS_HEIGHT_RES + 0.5 ) * step
			local h = sm.terrainTile.getHeightAt( uid, offsetX, offsetY, 0, rx, ry )
			if minHeight == nil or h < minHeight then minHeight = h end
			if maxHeight == nil or h > maxHeight then maxHeight = h end
			heightParts[#heightParts + 1] = encodeHeight( h )
		end
	end

	-- Objects are per cell rather than per sample, so this is a couple of calls
	-- per cell. Assets are the set pieces -- buildings, giant trees, the crashed
	-- ship -- while ordinary forest and boulders are harvestables. Both are just
	-- a uuid and a position, so they share one palette and stream and are told
	-- apart by uuid on the other side.
	local palette, paletteIndex, assetParts = {}, {}, {}

	local function collect( objects, offsetX, offsetY )
		if type( objects ) ~= "table" then return end
		for _, object in ipairs( objects ) do
			local key = tostring( object.uuid )
			local index = paletteIndex[key]
			if index == nil then
				palette[#palette + 1] = key
				index = #palette - 1
				paletteIndex[key] = index
			end
			assetParts[#assetParts + 1] = encodeAsset(
				index,
				offsetX * CELL + object.pos.x,
				offsetY * CELL + object.pos.y
			)
		end
	end

	for offsetY = 0, size - 1 do
		for offsetX = 0, size - 1 do
			local ok, assets = pcall( sm.terrainTile.getAssetsForCell, uid, offsetX, offsetY, 0 )
			if ok then collect( assets, offsetX, offsetY ) end
			local grown, harvestables =
				pcall( sm.terrainTile.getHarvestablesForCell, uid, offsetX, offsetY, 0 )
			if grown then collect( harvestables, offsetX, offsetY ) end
		end
	end

	return {
		materialSpan = materialSpan,
		clutterSpan = clutterSpan,
		clutter = table.concat( clutterParts ),
		colorSpan = colorSpan,
		heightSpan = heightSpan,
		material = table.concat( materialParts ),
		color = table.concat( colorParts ),
		height = table.concat( heightParts ),
		assetPalette = palette,
		assets = table.concat( assetParts ),
		assetCount = #assetParts,
		minHeight = minHeight,
		maxHeight = maxHeight
	}
end

function ScrapMapBakeAtlas()
	local database = GetTileDatabase()
	if database == nil then
		sm.log.warning( "SCRAPMAP_ATLAS_V1|error|tile database unavailable" )
		return
	end

	local entries = {}
	for key, info in pairs( database ) do
		entries[#entries + 1] = { uuid = key, info = info }
	end
	table.sort( entries, function( a, b ) return a.uuid < b.uuid end )

	-- The index is cheap and describes the whole registry, so rewrite it every
	-- load. It lets ScrapMap tell "not baked yet" from "no such tile".
	local index = {}
	for _, entry in ipairs( entries ) do
		index[#index + 1] = {
			uuid = entry.uuid,
			path = entry.info.path,
			size = entry.info.size,
			terrainType = entry.info.terrainType,
			poiType = entry.info.poiType
		}
	end
	sm.json.save( {
		schemaVersion = SCRAPMAP_ATLAS_VERSION,
		materialResolution = SCRAPMAP_ATLAS_MATERIAL_RES,
		colorResolution = SCRAPMAP_ATLAS_COLOR_RES,
		heightResolution = SCRAPMAP_ATLAS_HEIGHT_RES,
		cellSize = CELL,
		tiles = index
	}, INDEX_PATH )

	local signature = bakeSignature()
	local storage = sm.terrainGeneration.loadGameStorage( STORAGE_CHANNEL )
	if type( storage ) ~= "table" or storage.signature ~= signature then
		storage = { signature = signature, baked = {} }
	end
	local alreadyBaked = storage.baked or {}

	local baked, failed, pending, spent = 0, 0, 0, 0
	sm.log.info( "SCRAPMAP_ATLAS_V1|begin|" .. tostring( #entries ) )

	for _, entry in ipairs( entries ) do
		if not alreadyBaked[entry.uuid] then
			local size = entry.info.size or 1
			local cost = size * size
			if spent > 0 and spent + cost > SCRAPMAP_ATLAS_CELL_BUDGET then
				pending = pending + 1
			else
				local path = ATLAS_DIR .. entry.uuid .. ".json"
				-- The database is keyed by tostring( uid ); the terrain API
				-- needs the uuid object back.
				local parsed, uid = pcall( sm.uuid.new, entry.uuid )
				local ok, result = false, "invalid uuid key"
				if parsed then
					ok, result = pcall( sampleTile, uid, size )
				end
				if ok and result then
					sm.json.save( {
						schemaVersion = SCRAPMAP_ATLAS_VERSION,
						uuid = entry.uuid,
						path = entry.info.path,
						size = size,
						terrainType = entry.info.terrainType,
						poiType = entry.info.poiType,
						cellSize = CELL,
						materialSpan = result.materialSpan,
						clutterSpan = result.clutterSpan,
						clutterResolution = SCRAPMAP_ATLAS_CLUTTER_RES,
						clutterEncoding = "clutter-index-hex",
						clutter = result.clutter,
						colorSpan = result.colorSpan,
						heightSpan = result.heightSpan,
						minHeight = result.minHeight,
						maxHeight = result.maxHeight,
						materialEncoding = "surface-class-hex",
						encoding = "rgb565-hex",
						heightEncoding = "decimetre-biased-hex",
						assetEncoding = "palette-index-quarter-metre-hex",
						assetCount = result.assetCount,
						assetPalette = result.assetPalette,
						assets = result.assets,
						material = result.material,
						color = result.color,
						height = result.height
					}, path )
					alreadyBaked[entry.uuid] = true
					baked = baked + 1
					sm.log.info( "SCRAPMAP_ATLAS_V1|tile|" .. entry.uuid .. "|" .. tostring( size ) )
				else
					failed = failed + 1
					sm.log.warning( "SCRAPMAP_ATLAS_V1|fail|" .. entry.uuid .. "|" .. tostring( result ) )
				end
				-- Charge the budget either way so one broken tile cannot use up
				-- every load's allowance on its own.
				spent = spent + cost
			end
		end
	end

	if baked > 0 then
		storage.baked = alreadyBaked
		sm.terrainGeneration.saveGameStorage( STORAGE_CHANNEL, storage )
	end

	sm.log.info( "SCRAPMAP_ATLAS_V1|done|baked=" .. tostring( baked ) ..
		"|failed=" .. tostring( failed ) .. "|pending=" .. tostring( pending ) ..
		"|cells=" .. tostring( spent ) )
end
