package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.FrustumProjection;
import net.minecraft.client.render.Frustum;
import org.joml.Matrix4f;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(Frustum.class)
public abstract class OrthographicFrustumMixin {
    @Shadow @Final private Matrix4f positionProjectionMatrix;
    @Inject(method="coverBoxAroundSetPosition",at=@At("HEAD"),cancellable=true)
    private void tf$fixedWidth(int boxSize,CallbackInfoReturnable<Frustum> ci) {
        // Vanilla moves the camera backwards until an 8-block camera box fits.
        // This cannot terminate when the orthographic clip volume is narrower
        // than that box. Keep the actual clip volume and its original position;
        // regular visibility tests still apply. Inspect this frustum's matrix,
        // so a projection toggle or session stop cannot race the decision.
        if(FrustumProjection.hasFixedWidth(positionProjectionMatrix))ci.setReturnValue((Frustum)(Object)this);
    }
}
