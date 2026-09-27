package dev.teekasfigure.exporter;
import org.joml.Matrix4fc;
/** Affine (orthographic) clip volumes never get wider when receding the camera. */
public final class FrustumProjection {
    public static boolean hasFixedWidth(Matrix4fc positionProjection) {
        return (positionProjection.properties() & Matrix4fc.PROPERTY_AFFINE)!=0;
    }
}
