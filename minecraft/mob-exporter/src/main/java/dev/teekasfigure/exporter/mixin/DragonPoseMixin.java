package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.boss.dragon.EnderDragonEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(EnderDragonEntity.class)
public abstract class DragonPoseMixin {
    @Inject(method="getSegmentProperties",at=@At("HEAD"),cancellable=true)
    private void tf$segments(int segment,float delta,CallbackInfoReturnable<double[]> ci){var e=(EnderDragonEntity)(Object)this;if(Participant.owned(e))ci.setReturnValue(new double[]{e.getYaw()-180,0,0});}
}
