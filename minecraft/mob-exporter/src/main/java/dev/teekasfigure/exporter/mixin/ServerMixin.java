package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.WorldVideoPlayer;
import net.minecraft.server.MinecraftServer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import java.util.function.BooleanSupplier;
@Mixin(MinecraftServer.class)
public abstract class ServerMixin {
    @Inject(method="tick",at=@At("HEAD"))private void tf$heartbeat(BooleanSupplier supplier,CallbackInfo ci){dev.teekasfigure.exporter.PlaybackWatchdog.serverPulse();}
    @Inject(method="tick",at=@At("TAIL"))private void tf$tick(BooleanSupplier supplier,CallbackInfo ci){WorldVideoPlayer.tick((MinecraftServer)(Object)this);}
    @Inject(method="shutdown",at=@At("HEAD"))private void tf$stop(CallbackInfo ci){WorldVideoPlayer.stop((MinecraftServer)(Object)this);}
}
