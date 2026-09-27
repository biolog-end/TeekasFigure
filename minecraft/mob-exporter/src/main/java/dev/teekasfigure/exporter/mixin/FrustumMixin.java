package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.client.render.Frustum;
import net.minecraft.client.render.entity.EntityRenderer;
import net.minecraft.entity.Entity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(EntityRenderer.class)
public abstract class FrustumMixin {
    @Inject(method="shouldRender",at=@At("HEAD"),cancellable=true)private void tf$visible(Entity entity,Frustum frustum,double x,double y,double z,CallbackInfoReturnable<Boolean> ci){if(Participant.owned(entity))ci.setReturnValue(true);}
}
