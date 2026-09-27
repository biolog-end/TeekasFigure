package dev.teekasfigure.exporter.mixin;

import dev.teekasfigure.exporter.Participant;
import dev.teekasfigure.exporter.ParticipantCache;
import net.minecraft.entity.Entity;
import net.minecraft.text.Text;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(Entity.class)
public abstract class ParticipantCacheMixin implements ParticipantCache {
    @Unique private Text tf$lastName;
    @Unique private Participant.Info tf$info;

    @Override public Participant.Info teekasfigure$metadata(Text name) {
        if(tf$info==null || tf$lastName!=name){tf$lastName=name;tf$info=Participant.parse(name);}
        return tf$info;
    }
}
