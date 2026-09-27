package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.OrthoCamera;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.render.BackgroundRenderer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(BackgroundRenderer.class)
public abstract class FogMixin {
    @Inject(method="applyFog",at=@At("TAIL"))private static void tf$fog(CallbackInfo ci){if(OrthoCamera.active()){float far=OrthoCamera.farPlane();RenderSystem.setShaderFogStart(far);RenderSystem.setShaderFogEnd(far+16);}}
}
