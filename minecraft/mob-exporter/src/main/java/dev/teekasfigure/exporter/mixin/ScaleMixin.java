package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.LivingEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(LivingEntity.class)
public abstract class ScaleMixin {
    @Inject(method="getScale",at=@At("RETURN"),cancellable=true)private void tf$scale(CallbackInfoReturnable<Float> ci){if(Participant.owned((LivingEntity)(Object)this))ci.setReturnValue(Participant.scale((LivingEntity)(Object)this));}
}
