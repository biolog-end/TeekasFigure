package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.LivingEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(LivingEntity.class)
public abstract class TrackedPoseMixin {
    @Inject(method="updateTrackedPositionAndAngles",at=@At("HEAD"),cancellable=true)
    private void tf$pose(double x,double y,double z,float yaw,float pitch,int steps,CallbackInfo ci){
        var entity=(LivingEntity)(Object)this;if(!Participant.owned(entity))return;
        entity.refreshPositionAndAngles(x,y,z,yaw,pitch);entity.bodyYaw=yaw;entity.prevBodyYaw=yaw;entity.setHeadYaw(yaw);entity.prevHeadYaw=yaw;ci.cancel();
    }
}
