package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.WorldVideoPlayer;
import net.minecraft.server.PlayerManager;
import net.minecraft.server.network.ServerPlayerEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(PlayerManager.class)
public abstract class DisconnectMixin {
    @Inject(method="remove",at=@At("HEAD"))private void tf$restore(ServerPlayerEntity player,CallbackInfo ci){WorldVideoPlayer.ownerLeaving(player.getServer(),player.getUuid());}
}
