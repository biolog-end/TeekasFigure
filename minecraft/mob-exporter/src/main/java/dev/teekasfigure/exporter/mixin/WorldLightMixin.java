package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.OrthoCamera;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.render.WorldRenderer;
import org.joml.Vector3f;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(WorldRenderer.class)
public abstract class WorldLightMixin {
    // Lighting is set for the complete entity batch, before its vertex buffers are drawn.
    @Inject(method="render",at={@At(value="INVOKE",target="Lnet/minecraft/client/render/DiffuseLighting;enableForLevel()V",shift=At.Shift.AFTER),@At(value="INVOKE",target="Lnet/minecraft/client/render/DiffuseLighting;disableForLevel()V",shift=At.Shift.AFTER)})
    private void tf$lights(CallbackInfo ci){if(OrthoCamera.active()){
        double pitch=Math.toRadians(MinecraftClient.getInstance().gameRenderer.getCamera().getPitch());
        var up=new Vector3f(0,(float)Math.cos(pitch),(float)Math.sin(pitch));RenderSystem.setShaderLights(up,up);
    }}
}
