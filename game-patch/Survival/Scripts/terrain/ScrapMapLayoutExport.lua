-- ScrapMap world layout export.
-- Extracted from terrain_overworld.lua so the stock file stays byte-identical
-- to vanilla apart from one appended block.

function ScrapMapExportLayout()
	local terrainNames = {
		[TYPE_MEADOW] = "meadow",
		[TYPE_FOREST] = "forest",
		[TYPE_DESERT] = "desert",
		[TYPE_FIELD] = "field",
		[TYPE_BURNTFOREST] = "burnt",
		[TYPE_AUTUMNFOREST] = "autumn",
		[TYPE_LAKE] = "lake"
	}
	local poiNames = {}
	for name, value in pairs( _G ) do
		if type( name ) == "string" and string.sub( name, 1, 4 ) == "POI_" and type( value ) == "number" then
			poiNames[value] = name
		end
	end

	local cells = {}
	for y = g_cellData.bounds.yMin, g_cellData.bounds.yMax do
		for x = g_cellData.bounds.xMin, g_cellData.bounds.xMax do
			local uid = g_cellData.uid[y][x]
			local flags = g_cellData.flags[y][x]
			local poiType = GetPoiType( uid )
			local poiName = poiType and poiNames[poiType] or nil
			local poi = nil
			if poiType then
				poi = {
					kind = poiName and string.lower( poiName ) or "poi",
					label = poiName or ( "POI " .. tostring( poiType ) ),
					code = poiName or tostring( poiType )
				}
			end
			cells[#cells + 1] = {
				x = x,
				y = y,
				uuid = tostring( uid ),
				path = GetPath( uid ),
				-- Optional: only present while tile_database.lua carries the legacy map.
				legacyId = GetLegacyID and GetLegacyID( uid ) or nil,
				tileSize = GetSize( uid ),
				terrain = terrainNames[GetTerrainType( uid )] or "unknown",
				poi = poi,
				poiType = poiType,
				rotation = g_cellData.rotation[y][x],
				xOffset = g_cellData.xOffset[y][x],
				yOffset = g_cellData.yOffset[y][x],
				groupId = g_cellData.groupId and g_cellData.groupId[y] and g_cellData.groupId[y][x] or nil,
				roadMask = bit.band( flags, MASK_ROADS ),
				flags = flags
			}
		end
	end

	local worldId = "world-" .. tostring( g_world.id )
	sm.json.save( {
		schemaVersion = 1,
		worldId = worldId,
		seed = g_cellData.seed,
		cellSize = 64,
		bounds = {
			minX = g_cellData.bounds.xMin,
			maxX = g_cellData.bounds.xMax,
			minY = g_cellData.bounds.yMin,
			maxY = g_cellData.bounds.yMax
		},
		cells = cells
	}, "$SURVIVAL_DATA/ScrapMapLayout.json" )
	sm.log.info( "SCRAPMAP_LAYOUT_V1|" .. worldId .. "|" .. tostring( #cells ) )
end
