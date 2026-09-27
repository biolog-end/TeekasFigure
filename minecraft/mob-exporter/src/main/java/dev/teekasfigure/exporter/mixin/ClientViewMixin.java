package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.ViewProfile;
import net.minecraft.client.MinecraftClient;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(MinecraftClient.class)
public abstract class ClientViewMixin {
    @Inject(method="tick",at=@At("TAIL"))private void tf$view(CallbackInfo ci){var client=(MinecraftClient)(Object)this;dev.teekasfigure.exporter.PlaybackWatchdog.clientPulse(client.isPaused());ViewProfile.tick(client);}
    @Inject(method={"disconnect","stop"},at=@At("HEAD"))private void tf$restore(CallbackInfo ci){ViewProfile.restore();}
}
