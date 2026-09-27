package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.CapturePoses;
import net.minecraft.client.model.ModelPart;
import net.minecraft.client.render.entity.model.BatEntityModel;
import net.minecraft.entity.passive.BatEntity;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(BatEntityModel.class)
public abstract class BatModelPoseMixin {
    @Shadow @Final private ModelPart head;
    @Shadow @Final private ModelPart body;
    @Shadow @Final private ModelPart rightWing;
    @Shadow @Final private ModelPart leftWing;
    @Shadow @Final private ModelPart rightWingTip;
    @Shadow @Final private ModelPart leftWingTip;
    @Inject(method="setAngles(Lnet/minecraft/entity/passive/BatEntity;FFFFF)V",at=@At("TAIL"))
    private void tf$horizontalFlight(BatEntity bat,float limb,float distance,float age,float yaw,float pitch,CallbackInfo ci) {
        if(!CapturePoses.matches(bat,"bat_top_v2"))return;
        // Wings are XY planes in the vanilla mesh. Pitch the flying body 90
        // degrees so those planes face the exact top-down camera, rather than
        // exporting the tilted, folded snapshot of the flying animation.
        head.resetTransform();body.traverse().forEach(ModelPart::resetTransform);
        head.pitch=(float)Math.PI/2;body.pitch=(float)Math.PI/2;
        rightWing.yaw=leftWing.yaw=rightWingTip.yaw=leftWingTip.yaw=0;
    }
}
