-- ScrapMap POI photography sweep.
--
-- Photographs every point of interest from directly above using the game's own
-- camera, producing real pictures of the crash site, warehouses and ruins --
-- things the procedural atlas cannot draw, because it only samples terrain.
--
-- The camera alone is not enough. Terrain streams around the *player*, not the
-- camera, so a camera-only sweep photographs sky wherever the player is not:
-- roughly half of a 116-target run came back as skybox. So the sweep moves the
-- player too, through the game's own travel path -- the same one warehouses and
-- elevators use. SurvivalGame.sv_e_recreatePlayerInWorld calls world:loadCell on
-- the destination and recreates the character there once it is loaded, which is
-- exactly the guarantee the sweep needs.
--
-- Each target is visited in two hops. The first drops the player in high above
-- the tile, which forces the cell to load and lets its neighbours stream in; the
-- second puts the player at a known height above the ground -- specifically,
-- above the camera.
--
-- Nothing about the scene is measured here any more. The ground, the terrain
-- relief and the height of the tallest building all arrive in the request,
-- worked out from the baked atlas and the assets' own collision meshes.
--
-- They used to be raycast in-game, and it never once worked: sm.physics.raycast
-- returns a RaycastResult *userdata*, so a `type( result ) == "table"` guard is
-- always false. Three sweeps framed every shot from sea level and never pulled
-- the camera back for a single tower, reporting nothing worse than "miss".
--
-- Above the camera on purpose. The camera looks straight down, so a player
-- higher than it is outside the frame no matter how the view is cropped, and a
-- player a hundred and fifty metres up is out of reach of everything that would
-- like to kill it. The first version of this stood the player on the ground at
-- the tile's centre, which put a falling character in the middle of every
-- photograph and fed it to a warehouse full of robots.
--
-- Recreating the character resets its velocity, so the fall never accumulates
-- and never lands: each hop starts it again from zero.
--
-- The camera does not sit at the height that frames the tile exactly. A tower is
-- nearly as tall as that camera is high, so its roof is barely below the lens
-- and leans right out over the tile's edges. Instead the tile is measured for
-- what is standing on it and the camera pulls back proportionally; ScrapMap
-- crops the surplus away, so the stored photograph still lines up with the tile.
--
-- The handshake with ScrapMap is deliberately one-way. Lua logs that it is in
-- position and holds the pose for a fixed dwell; ScrapMap watches the log and
-- captures during that window. Lua cannot poll for an acknowledgement, because
-- sm.json.fileExists does not see files written during the same session.
--
-- Enabling: ScrapMap writes ScrapMapCapture.json and the sweep starts on the
-- next world load, which is when this file next looks for it. ScrapMap removes
-- the file once the sweep reports done, so it runs once rather than every load.

SCRAPMAP_SHOOT_VERSION = 4

-- Seconds to hold the framed pose. Covers ScrapMap's poll and capture; the log
-- line is emitted at the start so the capture lands mid-dwell.
SCRAPMAP_SHOOT_DWELL = 2.0
-- Seconds spent hovering after arrival, so the cells around the destination
-- stream in before the ground is measured and the shot is framed.
SCRAPMAP_SHOOT_SETTLE = 2.5
-- Seconds to let the second hop settle before the pose is announced, and how
-- long to keep waiting for the character to actually be above the camera.
SCRAPMAP_SHOOT_PERCH = 1.0
SCRAPMAP_SHOOT_PERCH_TIMEOUT = 5.0
-- Give up waiting for a travel to complete after this long and carry on. A
-- stalled target must not stall the whole sweep; ScrapMap's own frame guards
-- reject the shot if the scene never arrived.
SCRAPMAP_SHOOT_TRAVEL_TIMEOUT = 10.0
-- Vertical field of view used for every shot. Fixed so that every tile is
-- framed identically and the captured square maps to a known ground distance.
SCRAPMAP_SHOOT_FOV = 60.0

local REQUEST_PATH = "$SURVIVAL_DATA/ScrapMapCapture.json"
-- Where the first hop puts the player: clear of every clifftop, so the scene
-- streams in without the character ever being buried inside it.
local TRAVEL_ALTITUDE = 400
-- How high above the camera hovers during travel, so the tile is already on
-- screen by the time it finishes streaming.
local TRAVEL_CAMERA_LIFT = 40
-- How far above the camera the second hop parks the player. Everything below
-- the camera is in shot, so this is the whole reason the character stays out of
-- the photograph. It also has to outlast the fall: the player is in free fall
-- from the moment it is recreated, and terminal velocity is around 50 m/s, so
-- this is roughly six seconds of margin. It used to be 150, which measured out
-- at about two and a half seconds -- less than a cold cell load can take, and
-- the player hit the ground and died waiting for one.
local PLAYER_LIFT = 300
-- How far above the camera the character must actually be before the pose is
-- announced. Level with the lens is not behind it.
local PERCH_MARGIN = 10
-- Dropped from this height when the player is put back where they started.
local LANDING_CLEARANCE = 1.0
-- How many times its own height above a structure the camera should sit. Four
-- puts the roof a third of the way up the column, which reads as a plan view
-- rather than an oblique one.
local LEAN_TOLERANCE = 4.0
-- Ceiling on that pull-back, as a multiple of the height that frames the tile
-- exactly. Everything outside the tile is cropped away afterwards, so pulling
-- back spends real pixels: 2.5 keeps the kept part of a 1440-tall capture above
-- the 512 the atlas stores.
local MAX_PULL_BACK = 2.5
-- Height above the framing plane below which the player is hauled back up.
--
-- Generous on purpose, and checked often: at terminal velocity the player covers
-- 25 m between checks, and the rescue itself has to load a cell before it takes
-- effect. Catching the fall at 120 m rather than 60 leaves time for both.
local FALL_FLOOR = 120
-- Seconds between rescues, so one slow travel does not become a stream of them.
local FALL_RESCUE_INTERVAL = 0.5
-- Horizontal distance within which the character counts as having arrived.
-- Distinct point-of-interest tiles are at least a cell apart, so this cannot be
-- satisfied by the previous target's position.
local ARRIVE_RADIUS = 12
local CELL = 64

local g_shoot = nil
-- Last stats the server sent the local player, captured so a death during a
-- sweep can be explained rather than guessed at. Two runs ended with the player
-- dead and no way to tell whether it was the fall, a robot, or simply fifteen
-- minutes with the controls locked and no chance to eat.
local g_stats = nil

-- SurvivalPlayer lives in its own file and may not exist when this one loads,
-- so the wrap is attempted from the update rather than at load time.
local function installStatProbe()
	if g_scrapMapStatProbeInstalled or SurvivalPlayer == nil
		or SurvivalPlayer.client_onClientDataUpdate == nil then
		return
	end
	local original = SurvivalPlayer.client_onClientDataUpdate
	function SurvivalPlayer.client_onClientDataUpdate( self, data )
		original( self, data )
		if type( data ) == "table" and type( data.stats ) == "table"
			and sm.localPlayer.getPlayer() == self.player then
			g_stats = data.stats
		end
	end
	g_scrapMapStatProbeInstalled = true
end

local function statsText()
	if type( g_stats ) ~= "table" then
		return "?"
	end
	-- This build has no hunger or thirst: the stats are hp and breath. Breath is
	-- the one that matters here -- a player who lands in water with the sweep
	-- holding their controls cannot swim up.
	return string.format( "hp=%s/%s breath=%s/%s",
		tostring( g_stats.hp ), tostring( g_stats.maxhp ),
		tostring( g_stats.breath ), tostring( g_stats.maxbreath ) )
end

-- Height at which a tile of `metres` across exactly fills the frame vertically.
-- The captured square is the centre of the client area, so its side covers the
-- same ground distance on both axes.
local function cameraHeight( metres )
	local halfFov = math.rad( SCRAPMAP_SHOOT_FOV ) * 0.5
	return ( metres * 0.5 ) / math.tan( halfFov )
end

local function localCharacter()
	local player = sm.localPlayer.getPlayer()
	local character = player and player.character
	if character and sm.exists( character ) then
		return character
	end
	return nil
end

-- Asks the server to move the local player. Only the server may recreate a
-- character, so the client cannot do this itself.
local function requestTravel( game, x, y, z, direction )
	game.network:sendToServer( "sv_scrapMapShootTravel", {
		player = sm.localPlayer.getPlayer(),
		pos = sm.vec3.new( x, y, z ),
		dir = direction or sm.vec3.new( 0, 1, 0 )
	} )
end

local function arrivedAt( x, y )
	local character = localCharacter()
	if character == nil then
		return false
	end
	local position = character:getWorldPosition()
	local dx = position.x - x
	local dy = position.y - y
	return dx * dx + dy * dy <= ARRIVE_RADIUS * ARRIVE_RADIUS
end

-- Re-applied whenever the character changes, not just once at the start.
-- Recreating the character is how the sweep travels, and a fresh character
-- brings back the HUD, the player's own controls and the default camera unless
-- they are put back. Re-applying on every frame instead would mean asking the
-- camera to enter the same state forty times a second, which is worth avoiding.
local function holdPose()
	local character = localCharacter()
	-- Compared by identity rather than by id: a recreated character is a new
	-- userdata, which is exactly the event this needs to notice.
	if character ~= g_shoot.posedCharacter then
		g_shoot.posedCharacter = character
		sm.gui.hideGui( true )
		sm.localPlayer.setLockedControls( true )
		sm.camera.setCameraState( sm.camera.state.cutsceneTP )
		-- The player stands at the tile's centre for the exposure, which is the
		-- middle of the photograph. Hide it rather than photograph it.
		if character then
			pcall( character.setVisible, character, false )
		end
	end
	sm.camera.setFov( SCRAPMAP_SHOOT_FOV )
	sm.camera.setDirection( sm.vec3.new( 0, 0, -1 ) )
end

local function releasePose()
	local character = localCharacter()
	if character then
		pcall( character.setVisible, character, true )
	end
	sm.gui.hideGui( false )
	sm.localPlayer.setLockedControls( false )
	sm.camera.setCameraState( sm.camera.state.default )
end

local function targetCentre( target )
	local size = target.size or 1
	-- Cell coordinates address the cell's corner, so aim at the tile's middle.
	return ( target.x + size * 0.5 ) * CELL, ( target.y + size * 0.5 ) * CELL, size * CELL
end

local function finishSweep( game )
	-- Put the player back where the sweep found them before handing control
	-- back, so a sweep is not a one-way trip across the map.
	if g_shoot.origin then
		requestTravel( game, g_shoot.origin.x, g_shoot.origin.y,
			g_shoot.origin.z + LANDING_CLEARANCE, g_shoot.originDirection )
	end
	g_shoot.phase = "finished"
	releasePose()
	sm.log.info( "SCRAPMAP_SHOT_V1|done|" .. tostring( #g_shoot.targets ) )
end

local function enterTarget( game, index )
	g_shoot.index = index
	g_shoot.timer = 0
	g_shoot.ground = nil
	g_shoot.probed = nil
	g_shoot.lift = nil
	g_shoot.structure = nil
	if index > #g_shoot.targets then
		finishSweep( game )
		return
	end
	local target = g_shoot.targets[index]
	local centreX, centreY, metres = targetCentre( target )
	-- Nothing here needs measuring in-game, so the shot can be framed before the
	-- journey rather than after it. The camera then watches the tile stream in
	-- from the position it will photograph from, instead of staring at sky from
	-- four hundred metres for most of every cycle.
	g_shoot.ground = target.groundHeight or 0
	g_shoot.probed = target.groundHeight and "atlas" or "sealevel"
	g_shoot.structure = math.max( target.reliefHeight or 0, target.structureHeight or 0 )
	local exact = cameraHeight( metres )
	g_shoot.lift = math.max( exact, math.min( g_shoot.structure * LEAN_TOLERANCE,
		exact * MAX_PULL_BACK ) )
	requestTravel( game, centreX, centreY, TRAVEL_ALTITUDE )
	g_shoot.phase = "travel"
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

	-- The sweep teleports the player. On someone else's server that would drag
	-- our character around their world for ten minutes, so refuse to start.
	-- Tested against false rather than falsiness: if the flag is not visible
	-- from here at all, a singleplayer sweep should still run.
	if sm.isHost == false then
		sm.log.info( "SCRAPMAP_SHOT_V1|idle|not the host; the sweep moves the player" )
		return
	end

	g_shoot = {
		targets = request.targets,
		index = 0,
		timer = 0,
		phase = "start"
	}
	sm.log.info( "SCRAPMAP_SHOT_V1|begin|" .. tostring( #request.targets ) )
end

-- Drives the sweep from the client update, alongside the telemetry provider.
local function shootUpdate( game, dt )
	if g_shoot == nil or g_shoot.phase == "finished" then
		return
	end

	if g_shoot.phase == "start" then
		local character = localCharacter()
		if character == nil then
			return -- The world is still loading; there is nothing to move yet.
		end
		g_shoot.origin = character:getWorldPosition()
		g_shoot.originDirection = character:getDirection()
		holdPose()
		enterTarget( game, 1 )
		return
	end

	holdPose()
	g_shoot.timer = g_shoot.timer + dt
	g_shoot.rescueTimer = ( g_shoot.rescueTimer or 0 ) + dt

	local target = g_shoot.targets[g_shoot.index]
	if target == nil then
		return
	end
	local centreX, centreY, metres = targetCentre( target )

	-- The player is in free fall for the whole sweep, and only the next travel
	-- stops it. If one is slow -- a cold cell load is easily seconds -- the fall
	-- reaches the ground and kills the player, which ends the sweep and leaves
	-- the camera photographing sky wherever the corpse is not. Catch it.
	local falling = localCharacter()
	if falling and g_shoot.rescueTimer >= FALL_RESCUE_INTERVAL then
		local floor = ( g_shoot.ground or 0 ) + ( g_shoot.lift or 0 ) + FALL_FLOOR
		if falling:getWorldPosition().z < floor then
			g_shoot.rescueTimer = 0
			requestTravel( game, centreX, centreY,
				( g_shoot.ground or 0 ) + ( g_shoot.lift or 0 ) + PLAYER_LIFT )
			sm.log.warning( "SCRAPMAP_SHOT_V1|rescue|" .. tostring( target.uuid ) )
		end
	end

	local height = ( g_shoot.ground or 0 ) + ( g_shoot.lift or cameraHeight( metres ) )
	sm.camera.setPosition( sm.vec3.new( centreX, centreY, height ) )

	if g_shoot.phase == "travel" then
		if arrivedAt( centreX, centreY ) then
			g_shoot.phase = "settle"
			g_shoot.timer = 0
		elseif g_shoot.timer >= SCRAPMAP_SHOOT_TRAVEL_TIMEOUT then
			sm.log.warning( "SCRAPMAP_SHOT_V1|slow|" .. tostring( target.uuid ) )
			g_shoot.phase = "settle"
			g_shoot.timer = 0
		end
		return
	end

	if g_shoot.phase == "settle" then
		if g_shoot.timer < SCRAPMAP_SHOOT_SETTLE then
			return
		end
		-- The ground comes from the baked atlas, not from a raycast. The game
		-- samples every tile's height when it bakes the map, which is exact and
		-- needs no physics; casting for it from the air is what silently failed
		-- for three sweeps and framed the entire map from sea level.
		g_shoot.ground = target.groundHeight or 0
		g_shoot.probed = target.groundHeight and "atlas" or "sealevel"
		-- Terrain relief counts as structure: a hill leans out over a top-down
		-- frame exactly the way a tower does.
		local structure = math.max( target.reliefHeight or 0, target.structureHeight or 0 )
		local exact = cameraHeight( metres )
		-- Far enough back that the tile's buildings stand up rather than lean
		-- out, but never so far that the crop costs more than it is worth.
		g_shoot.lift = math.max( exact, math.min( structure * LEAN_TOLERANCE,
			exact * MAX_PULL_BACK ) )
		g_shoot.structure = structure
		-- Park the player above where the camera is about to be, so it is behind
		-- the lens rather than in the middle of the picture.
		requestTravel( game, centreX, centreY,
			g_shoot.ground + g_shoot.lift + PLAYER_LIFT )
		g_shoot.phase = "perch"
		g_shoot.timer = 0
		return
	end

	local lift = g_shoot.lift or cameraHeight( metres )
	local height = ( g_shoot.ground or 0 ) + lift
	sm.camera.setPosition( sm.vec3.new( centreX, centreY, height ) )
	-- Ground distance the full frame height now covers. It is >= the tile
	-- whenever the camera pulled back, and ScrapMap crops the difference away so
	-- the stored photograph still lines up with the tile exactly.
	local covered = metres * lift / cameraHeight( metres )

	if g_shoot.phase == "perch" then
		if g_shoot.timer < SCRAPMAP_SHOOT_PERCH then
			return
		end
		-- Do not announce the pose until the character is genuinely behind the
		-- camera, or it lands in the middle of the photograph. Still falling
		-- from the first hop satisfies this on its own, which is fine: the
		-- requirement is where the character is, not which hop put it there.
		local character = localCharacter()
		-- No character at all is no character in the picture, so treat a dead or
		-- respawning player as clear rather than stalling on it.
		local clearance = character and ( character:getWorldPosition().z - height )
			or PLAYER_LIFT
		if clearance < PERCH_MARGIN and g_shoot.timer < SCRAPMAP_SHOOT_PERCH_TIMEOUT then
			return
		end
		g_shoot.phase = "hold"
		g_shoot.timer = 0
		-- `covered` is what ScrapMap crops against. The rest is diagnostic:
		-- clearance tells a photograph with a character in it apart from one
		-- that was simply framed wrong, and structure explains the pull-back.
		sm.log.info( string.format(
			"SCRAPMAP_SHOT_V1|ready|%s|%d|%d|%d|%.2f|%.2f|%.1f|%s|%.1f|%.1f|%s",
			tostring( target.uuid ), target.x, target.y, target.size or 1, metres,
			covered, height, tostring( g_shoot.probed ), clearance,
			g_shoot.structure or 0, statsText() ) )
		return
	end

	if g_shoot.phase == "hold" and g_shoot.timer >= SCRAPMAP_SHOOT_DWELL then
		enterTarget( game, g_shoot.index + 1 )
	end
end

-- Server half of the sweep. Only the server may recreate a character, and
-- sv_e_recreatePlayerInWorld is the game's own travel path: it loads the
-- destination cell first and recreates the character in the callback, which is
-- what makes the destination actually be there when the camera looks.
--
-- Defined outside the install guard on purpose. Assigning a method is
-- idempotent, unlike wrapping one, so it does not need protecting -- and it
-- must not be skipped if this file is executed a second time.
function SurvivalGame.sv_scrapMapShootTravel( self, params )
	local player = params and params.player
	if player == nil then
		return
	end
	local character = player.character
	if character == nil or not sm.exists( character ) then
		sm.log.warning( "SCRAPMAP_SHOT_V1|travel refused: the player has no character" )
		return
	end
	sm.event.sendToGame( "sv_e_recreatePlayerInWorld", {
		player = player,
		world = character:getWorld(),
		pos = params.pos,
		dir = params.dir
	} )
end

if not g_scrapMapShootInstalled then
	local originalClientOnUpdate = SurvivalGame.client_onUpdate

	function SurvivalGame.client_onUpdate( self, dt )
		originalClientOnUpdate( self, dt )

		-- The request is read once, on the first client frame: by then the world
		-- exists, and re-reading it every frame would be pointless work.
		installStatProbe()
		if not self.cl.scrapMapShootChecked then
			self.cl.scrapMapShootChecked = true
			shootLoad()
		end
		shootUpdate( self, dt )
	end

	g_scrapMapShootInstalled = true
	sm.log.info( "SCRAPMAP_SHOT_V1|installed" )
end
