package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.OrthoCamera;
import net.minecraft.client.render.GameRenderer;
import org.joml.Matrix4f;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(GameRenderer.class)
public abstract class ProjectionMixin {
    @Inject(method="getBasicProjectionMatrix",at=@At("HEAD"),cancellable=true)private void tf$projection(double fov,CallbackInfoReturnable<Matrix4f> ci){var projection=OrthoCamera.projection(fov);if(projection!=null)ci.setReturnValue(projection);}
}
