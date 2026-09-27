package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.CapturePoses;
import net.minecraft.client.model.ModelPart;
import net.minecraft.client.render.entity.model.AxolotlEntityModel;
import net.minecraft.entity.passive.AxolotlEntity;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
@Mixin(AxolotlEntityModel.class)
public abstract class AxolotlModelPoseMixin {
    @Shadow @Final private ModelPart body;
    @Shadow @Final private ModelPart tail;
    @Shadow @Final private ModelPart leftHindLeg;
    @Shadow @Final private ModelPart rightHindLeg;
    @Shadow @Final private ModelPart leftFrontLeg;
    @Shadow @Final private ModelPart rightFrontLeg;
    @Shadow @Final private ModelPart topGills;
    @Shadow @Final private ModelPart leftGills;
    @Shadow @Final private ModelPart rightGills;
    @Inject(method="setAngles(Lnet/minecraft/entity/passive/AxolotlEntity;FFFFF)V",at=@At("HEAD"),cancellable=true)
    private void tf$spreadLegs(AxolotlEntity entity,float limb,float distance,float age,float yaw,float pitch,CallbackInfo ci) {
        if(!CapturePoses.matches(entity,"axolotl_top_v2"))return;
        body.traverse().forEach(ModelPart::resetTransform);
        // Vanilla standing leg angles, without a history-dependent animation
        // lerp. A small fin roll exposes genuinely zero-thickness polygons.
        angles(leftHindLeg,1.1f,1,0);angles(rightHindLeg,1.1f,-1,0);
        angles(leftFrontLeg,.8f,2.3f,-.5f);angles(rightFrontLeg,.8f,-2.3f,.5f);
        angles(tail,0,-.1f,.25f);angles(topGills,.65f,0,0);
        angles(leftGills,0,-.65f,-.25f);angles(rightGills,0,.65f,.25f);
        ci.cancel();
    }
    private static void angles(ModelPart part,float pitch,float yaw,float roll){part.pitch=pitch;part.yaw=yaw;part.roll=roll;}
}
