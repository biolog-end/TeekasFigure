package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.client.render.entity.EntityRenderDispatcher;
import net.minecraft.entity.Entity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
import org.spongepowered.asm.mixin.injection.invoke.arg.Args;
@Mixin(EntityRenderDispatcher.class)
public abstract class LightMixin {
    @Inject(method="getLight",at=@At("HEAD"),cancellable=true)private void tf$light(Entity entity,float delta,CallbackInfoReturnable<Integer> ci){if(Participant.owned(entity))ci.setReturnValue(0xF000F0);}
    @ModifyArgs(method="render",at=@At(value="INVOKE",target="Lnet/minecraft/client/render/entity/EntityRenderer;render(Lnet/minecraft/entity/Entity;FFLnet/minecraft/client/util/math/MatrixStack;Lnet/minecraft/client/render/VertexConsumerProvider;I)V"))
    private void tf$stillFrame(Args args){if(Participant.owned(args.<Entity>get(0)))args.set(2,0.0f);}
}
