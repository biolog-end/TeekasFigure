package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.client.render.entity.LivingEntityRenderer;
import net.minecraft.entity.LivingEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(LivingEntityRenderer.class)
public abstract class HideLabelMixin {
    @Inject(method="hasLabel(Lnet/minecraft/entity/LivingEntity;)Z",at=@At("HEAD"),cancellable=true)private void tf$hide(LivingEntity entity,CallbackInfoReturnable<Boolean> ci){if(Participant.owned(entity))ci.setReturnValue(false);}
}
