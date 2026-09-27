package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.Entity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(Entity.class)
public abstract class VisibleMixin {
    @Inject(method="shouldRender(D)Z",at=@At("HEAD"),cancellable=true)private void tf$visible(double distance,CallbackInfoReturnable<Boolean> ci){if(Participant.owned((Entity)(Object)this))ci.setReturnValue(true);}
}
