package dev.teekasfigure.exporter.mixin;
import dev.teekasfigure.exporter.Participant;
import net.minecraft.entity.Entity;
import net.minecraft.nbt.NbtCompound;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;
@Mixin(Entity.class)
public abstract class SessionSaveMixin {
    @Inject(method={"saveNbt","saveSelfNbt"},at=@At("HEAD"),cancellable=true)private void tf$temporary(NbtCompound nbt,CallbackInfoReturnable<Boolean> ci){if(Participant.owned((Entity)(Object)this))ci.setReturnValue(false);}
    @Inject(method="shouldSave",at=@At("HEAD"),cancellable=true)private void tf$skipSave(CallbackInfoReturnable<Boolean> ci){if(Participant.owned((Entity)(Object)this))ci.setReturnValue(false);}
}
