-- ScrapMap POI photography sweep.
--
-- Points the camera straight down at each point of interest in turn and holds
-- it still long enough for ScrapMap to capture the window. The result is a real
-- top-down photograph of the crash site, warehouses, ruins and so on, which the
-- procedural atlas cannot produce because it only samples terrain.
--
-- The handshake is deliberately one-way. Lua logs that it is in position and
-- then holds the pose for a fixed dwell; ScrapMap watches the log and captures
-- during that window. Lua cannot check for an acknowledgement file, because
-- sm.json.fileExists does not see files written during the same session.
--
-- Enabling: ScrapMap writes ScrapMapCapture.json before the game starts, since
-- for the same reason a file created mid-session would be invisible. ScrapMap
-- removes it once the sweep is done, so it runs once rather than every load.

SCRAPMAP_SHOOT_VERSION = 1

-- Seconds to hold each pose. Covers terrain streaming plus ScrapMap's poll and
-- capture; the log line is emitted at the start so the capture lands mid-dwell.
SCRAPMAP_SHOOT_DWELL = 3.0
-- Vertical field of view used for every shot. Fixed so that every tile is
-- framed identically and the captured square maps to a known ground distance.
SCRAPMAP_SHOOT_FOV = 60.0

local REQUEST_PATH = "$SURVIVAL_DATA/ScrapMapCapture.json"
-- Vertical span the ground probe searches. Comfortably brackets survival
-- terrain, which runs from lake beds to clifftops.
local GROUND_PROBE_TOP = 800
local GROUND_PROBE_BOTTOM = -200
local CELL = 64

local g_shoot = nil

-- Height at which a tile of `metres` across exactly fills the frame vertically.
-- The captured square is the centre of the client area, so its side covers the
-- same ground distance on both axes.
local function cameraHeight( metres )
	local halfFov = math.rad( SCRAPMAP_SHOOT_FOV ) * 0.5
	return ( metres * 0.5 ) / math.tan( halfFov )
end

local function shootLoad()
	-- Deliberately no sm.json.fileExists gate: it does not see files written
	-- after the game started, which is exactly when ScrapMap writes the
	-- request. Try to open it and let that answer the question, and say so
	-- either way so a sweep that does not start is diagnosable from the log.
	local ok, request = pcall( sm.json.open, REQUEST_PATH )
	if not ok or type( request ) ~= "table" or type( request.targets ) ~= "table"
		or #request.targets == 0 then
		sm.log.info( "SCRAPMAP_SHOT_V1|idle|no readable capture request" )
		return
	end

	g_shoot = {
		targets = request.targets,
		index = 0,
		timer = 0,
		announced = false
	}
	sm.log.info( "SCRAPMAP_SHOT_V1|begin|" .. tostring( #request.targets ) )
end

-- Drives the sweep from the client update, alongside the telemetry provider.
local function shootUpdate( dt )
	if g_shoot == nil then
		return
	end

	if g_shoot.index > #g_shoot.targets then
		return
	end

	if g_shoot.index == 0 then
		-- Take the camera before the first shot and keep it for the sweep.
		g_shoot.index = 1
		g_shoot.timer = 0
		g_shoot.announced = false
		g_shoot.ground = nil
		sm.gui.hideGui( true )
		sm.localPlayer.setLockedControls( true )
		sm.camera.setCameraState( sm.camera.state.cutsceneTP )
		sm.render.setCinematic( true )
	end

	local target = g_shoot.targets[g_shoot.index]
	if target == nil then
		return
	end

	local size = target.size or 1
	local metres = size * CELL
	-- Cell coordinates address the cell's corner, so aim at the tile's middle.
	local centreX = ( target.x + size * 0.5 ) * CELL
	local centreY = ( target.y + size * 0.5 ) * CELL
	-- Framing has to clear the local terrain, not sea level. A fixed altitude
	-- puts the camera inside a hill or under a lake, which comes back as blank
	-- fog or open water rather than the tile.
	if g_shoot.ground == nil then
		local from = sm.vec3.new( centreX, centreY, GROUND_PROBE_TOP )
		local to = sm.vec3.new( centreX, centreY, GROUND_PROBE_BOTTOM )
		-- pcall returns its own success flag first, then the call's results.
		local called, hit, result = pcall( sm.physics.raycast, from, to )
		if called and hit and type( result ) == "table" and result.pointWorld then
			g_shoot.ground = result.pointWorld.z
		else
			-- Sea level is a poor guess, but better than skipping the shot.
			g_shoot.ground = target.groundHeight or 0
		end
	end
	local height = g_shoot.ground + cameraHeight( metres )

	sm.camera.setFov( SCRAPMAP_SHOOT_FOV )
	sm.camera.setPosition( sm.vec3.new( centreX, centreY, height ) )
	sm.camera.setDirection( sm.vec3.new( 0, 0, -1 ) )

	if not g_shoot.announced then
		g_shoot.announced = true
		sm.log.info( string.format(
			"SCRAPMAP_SHOT_V1|ready|%s|%d|%d|%d|%.2f|%.1f",
			tostring( target.uuid ), target.x, target.y, size, metres, height ) )
	end

	g_shoot.timer = g_shoot.timer + dt
	if g_shoot.timer >= SCRAPMAP_SHOOT_DWELL then
		g_shoot.timer = 0
		g_shoot.announced = false
		g_shoot.ground = nil
		g_shoot.index = g_shoot.index + 1
		if g_shoot.index > #g_shoot.targets then
			sm.gui.hideGui( false )
			sm.localPlayer.setLockedControls( false )
			sm.camera.setCameraState( sm.camera.state.default )
			sm.render.setCinematic( false )
			sm.log.info( "SCRAPMAP_SHOT_V1|done|" .. tostring( #g_shoot.targets ) )
		end
	end
end

if not g_scrapMapShootInstalled then
	local originalClientOnUpdate = SurvivalGame.client_onUpdate

	function SurvivalGame.client_onUpdate( self, dt )
		originalClientOnUpdate( self, dt )

		-- The request is read once, on the first client frame: by then the world
		-- exists, and re-reading it every frame would be pointless work.
		if not self.cl.scrapMapShootChecked then
			self.cl.scrapMapShootChecked = true
			shootLoad()
		end
		shootUpdate( dt )
	end

	g_scrapMapShootInstalled = true
	sm.log.info( "SCRAPMAP_SHOT_V1|installed" )
end
