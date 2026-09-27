package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.boss.WitherEntity;
import net.minecraft.server.network.ServerPlayerEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(WitherEntity.class)
public abstract class WitherBarMixin {
    @Inject(method="onStartedTrackingBy",at=@At("HEAD"),cancellable=true)
    private void tf$noPlaybackBar(ServerPlayerEntity player,CallbackInfo ci){if(Participant.owned((WitherEntity)(Object)this))ci.cancel();}
}
