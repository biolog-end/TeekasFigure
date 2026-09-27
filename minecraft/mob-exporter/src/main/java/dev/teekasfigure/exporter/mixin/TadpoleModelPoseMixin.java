package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.CapturePoses;
import net.minecraft.client.model.ModelPart;
import net.minecraft.client.render.entity.model.TadpoleEntityModel;
import net.minecraft.entity.passive.TadpoleEntity;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(TadpoleEntityModel.class)
public abstract class TadpoleModelPoseMixin {
    @Shadow @Final private ModelPart tail;
    @Inject(method="setAngles(Lnet/minecraft/entity/passive/TadpoleEntity;FFFFF)V",at=@At("TAIL"))
    private void tf$fin(TadpoleEntity entity,float limb,float distance,float age,float yaw,float pitch,CallbackInfo ci) {
        tail.roll=CapturePoses.matches(entity,"tadpole_top_v2")?.3f:0;
    }
}
