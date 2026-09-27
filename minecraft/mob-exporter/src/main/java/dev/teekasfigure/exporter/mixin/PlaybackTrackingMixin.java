package dev.teekasfigure.exporter.mixin;

import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.Entity;
import net.minecraft.entity.mob.MobEntity;
import net.minecraft.server.network.EntityTrackerEntry;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(EntityTrackerEntry.class)
public abstract class PlaybackTrackingMixin {
    @Shadow @Final private Entity entity;
    @Shadow @Final @Mutable private int tickInterval;

    @Inject(method="<init>",at=@At("RETURN"))
    private void tf$videoRate(CallbackInfo ci) {
        // Vanilla's mob-specific intervals otherwise make 12/15-fps scenes
        // arrive as ~6-fps jumps even when every frame is ready on the server.
        if(entity instanceof MobEntity && Participant.owned(entity))tickInterval=1;
    }
}
