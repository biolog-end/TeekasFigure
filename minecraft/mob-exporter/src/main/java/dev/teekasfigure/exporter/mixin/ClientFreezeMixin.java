package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import dev.teekasfigure.exporter.CapturePoses;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.entity.Entity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(ClientWorld.class)
public abstract class ClientFreezeMixin {
    @Inject(method="tickEntity",at=@At("HEAD"),cancellable=true)private void tf$freeze(Entity entity,CallbackInfo ci){if(entity instanceof net.minecraft.entity.mob.MobEntity mob && Participant.owned(entity)){
        if(Participant.metadata(entity).pose().equals("bat_flying"))CapturePoses.apply(mob,"bat_flying");ci.cancel();
    }}
}
