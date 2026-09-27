package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.Entity;
import net.minecraft.server.world.ServerWorld;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(ServerWorld.class)
public abstract class WorldFreezeMixin {
    @Inject(method="tickEntity",at=@At("HEAD"),cancellable=true)private void tf$freeze(Entity entity,CallbackInfo ci){if(Participant.owned(entity))ci.cancel();}
}
