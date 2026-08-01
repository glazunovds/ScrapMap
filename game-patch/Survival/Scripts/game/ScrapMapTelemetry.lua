-- Local, read-only telemetry provider for the ScrapMap overlay.
-- Loaded only when Scrap Mechanic runs with its supported -dev flag.

if not g_scrapMapTelemetryInstalled then
	local originalClientOnUpdate = SurvivalGame.client_onUpdate

	function SurvivalGame.client_onUpdate( self, dt )
		originalClientOnUpdate( self, dt )

		self.cl.scrapMapTelemetryTimer = ( self.cl.scrapMapTelemetryTimer or 0.0 ) + dt
		if self.cl.scrapMapTelemetryTimer < 0.25 then
			return
		end
		self.cl.scrapMapTelemetryTimer = 0.0

		local localPlayer = sm.localPlayer.getPlayer()
		for _, player in ipairs( sm.player.getAllPlayers() ) do
			local character = player.character
			if character and sm.exists( character ) then
				local position = character:getWorldPosition()
				local direction = character:getDirection()
				local world = character:getWorld()
				local safeName = string.gsub( tostring( player.name ), "|", "_" )
				local message = string.format(
					"SCRAPMAP_TELEMETRY_V1|%s|%s|%s|%s|%.5f|%.5f|%.5f|%.5f|%.5f|%.5f",
					tostring( player.id ), player == localPlayer and "1" or "0", safeName,
					tostring( world.id ), position.x, position.y, position.z,
					direction.x, direction.y, direction.z
				)
				sm.log.info( message )
			end
		end
	end

	g_scrapMapTelemetryInstalled = true
	sm.log.info( "SCRAPMAP_TELEMETRY_PROVIDER_V1|ready" )
end
