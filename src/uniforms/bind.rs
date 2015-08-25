/*!

Handles binding uniforms to the OpenGL state machine.

*/
use gl;

use BufferExt;
use BufferSliceExt;
use DrawError;
use ProgramExt;
use UniformsExt;
use RawUniformValue;
use TextureExt;

use uniforms::Uniforms;
use uniforms::UniformValue;
use uniforms::SamplerBehavior;

use context::CommandContext;
use buffer::Inserter;
use ContextExt;

use utils::bitsfield::Bitsfield;

use vertex::MultiVerticesSource;

use program;
use context;
use version::Version;
use version::Api;

impl<U> UniformsExt for U where U: Uniforms {
    fn bind_uniforms<'a, P>(&'a self, mut ctxt: &mut CommandContext, program: &P,
                            fences: &mut Vec<Inserter<'a>>)
                            -> Result<(), DrawError>
                            where P: ProgramExt
    {
        let mut texture_bind_points = Bitsfield::new();
        let mut uniform_buffer_bind_points = Bitsfield::new();
        let mut shared_storage_buffer_bind_points = Bitsfield::new();

        let mut visiting_result = Ok(());
        self.visit_values(|name, value| {
            if visiting_result.is_err() { return; }

            if let Some(uniform) = program.get_uniform(name) {
                assert!(uniform.size.is_none(), "Uniform arrays not supported yet");

                if !value.is_usable_with(&uniform.ty) {
                    visiting_result = Err(DrawError::UniformTypeMismatch {
                        name: name.to_owned(),
                        expected: uniform.ty,
                    });
                    return;
                }

                match bind_uniform(&mut ctxt, &value, program, uniform.location,
                                   &mut texture_bind_points, name)
                {
                    Ok(_) => (),
                    Err(e) => {
                        visiting_result = Err(e);
                        return;
                    }
                };

            } else if let Some(block) = program.get_uniform_blocks().get(name) {
                let fence = match bind_uniform_block(&mut ctxt, &value, block,
                                                     program, &mut uniform_buffer_bind_points, name)
                {
                    Ok(f) => f,
                    Err(e) => {
                        visiting_result = Err(e);
                        return;
                    }
                };

                if let Some(fence) = fence {
                    fences.push(fence);
                }

            } else if let Some(block) = program.get_shader_storage_blocks().get(name) {
                let fence = match bind_shared_storage_block(&mut ctxt, &value, block, program,
                                                            &mut shared_storage_buffer_bind_points,
                                                            name)
                {
                    Ok(f) => f,
                    Err(e) => {
                        visiting_result = Err(e);
                        return;
                    }
                };

                if let Some(fence) = fence {
                    fences.push(fence);
                }
            }
        });

        visiting_result
    }
}

fn bind_uniform_block<'a, P>(ctxt: &mut context::CommandContext, value: &UniformValue<'a>,
                             block: &program::UniformBlock,
                             program: &P, buffer_bind_points: &mut Bitsfield, name: &str)
                             -> Result<Option<Inserter<'a>>, DrawError>
                             where P: ProgramExt
{
    match value {
        &UniformValue::Block(buffer, ref layout) => {
            match layout(block) {
                Ok(_) => (),
                Err(e) => {
                    return Err(DrawError::UniformBlockLayoutMismatch {
                        name: name.to_owned(),
                        err: e,
                    });
                }
            }

            let bind_point = buffer_bind_points.get_unused().expect("Not enough buffer units");
            buffer_bind_points.set_used(bind_point);

            assert!(buffer.get_offset_bytes() == 0);     // TODO: not implemented
            let fence = buffer.add_fence();
            let binding = block.binding as gl::types::GLuint;

            buffer.prepare_and_bind_for_uniform(ctxt, bind_point as gl::types::GLuint);
            program.set_uniform_block_binding(ctxt, binding, bind_point as gl::types::GLuint);

            Ok(fence)
        },
        _ => {
            Err(DrawError::UniformValueToBlock { name: name.to_owned() })
        }
    }
}

fn bind_shared_storage_block<'a, P>(ctxt: &mut context::CommandContext, value: &UniformValue<'a>,
                                    block: &program::UniformBlock,
                                    program: &P, buffer_bind_points: &mut Bitsfield, name: &str)
                                    -> Result<Option<Inserter<'a>>, DrawError>
                                    where P: ProgramExt
{
    match value {
        &UniformValue::Block(buffer, ref layout) => {
            match layout(block) {
                Ok(_) => (),
                Err(e) => {
                    return Err(DrawError::UniformBlockLayoutMismatch {
                        name: name.to_owned(),
                        err: e,
                    });
                }
            }

            let bind_point = buffer_bind_points.get_unused().expect("Not enough buffer units");
            buffer_bind_points.set_used(bind_point);

            assert!(buffer.get_offset_bytes() == 0);     // TODO: not implemented
            let fence = buffer.add_fence();
            let binding = block.binding as gl::types::GLuint;

            buffer.prepare_and_bind_for_shared_storage(ctxt, bind_point as gl::types::GLuint);
            program.set_shader_storage_block_binding(ctxt, binding, bind_point as gl::types::GLuint);

            Ok(fence)
        },
        _ => {
            Err(DrawError::UniformValueToBlock { name: name.to_owned() })
        }
    }
}

fn bind_uniform<P>(ctxt: &mut context::CommandContext,
                   value: &UniformValue, program: &P, location: gl::types::GLint,
                   texture_bind_points: &mut Bitsfield, name: &str)
                   -> Result<(), DrawError> where P: ProgramExt
{
    assert!(location >= 0);

    match *value {
        UniformValue::Block(_, _) => {
            Err(DrawError::UniformBufferToValue {
                name: name.to_owned(),
            })
        },
        UniformValue::Bool(val) => {
            // Booleans get passed as integers.
            program.set_uniform(ctxt, location, &RawUniformValue::SignedInt(val as i32));
            Ok(())
        },
        UniformValue::SignedInt(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::SignedInt(val));
            Ok(())
        },
        UniformValue::UnsignedInt(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::UnsignedInt(val));
            Ok(())
        },
        UniformValue::Float(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Float(val));
            Ok(())
        },
        UniformValue::Mat2(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Mat2(val));
            Ok(())
        },
        UniformValue::Mat3(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Mat3(val));
            Ok(())
        },
        UniformValue::Mat4(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Mat4(val));
            Ok(())
        },
        UniformValue::Vec2(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Vec2(val));
            Ok(())
        },
        UniformValue::Vec3(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Vec3(val));
            Ok(())
        },
        UniformValue::Vec4(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::Vec4(val));
            Ok(())
        },
        UniformValue::IntVec2(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::IntVec2(val));
            Ok(())
        },
        UniformValue::IntVec3(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::IntVec3(val));
            Ok(())
        },
        UniformValue::IntVec4(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::IntVec4(val));
            Ok(())
        },
        UniformValue::UnsignedIntVec2(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::UnsignedIntVec2(val));
            Ok(())
        },
        UniformValue::UnsignedIntVec3(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::UnsignedIntVec3(val));
            Ok(())
        },
        UniformValue::UnsignedIntVec4(val) => {
            program.set_uniform(ctxt, location, &RawUniformValue::UnsignedIntVec4(val));
            Ok(())
        },
        UniformValue::Texture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture1d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture2d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture2dMultisample(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture2dMultisample(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture2dMultisample(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture2dMultisample(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture2dMultisample(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture3d(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture1dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture2dArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Texture2dMultisampleArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbTexture2dMultisampleArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralTexture2dMultisampleArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedTexture2dMultisampleArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthTexture2dMultisampleArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::Cubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthCubemap(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::SrgbCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::CompressedSrgbCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::IntegralCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::UnsignedCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::DepthCubemapArray(texture, sampler) => {
            bind_texture_uniform(ctxt, &**texture, sampler, location, program, texture_bind_points)
        },
        UniformValue::BufferTexture(texture) => {
            bind_texture_uniform(ctxt, &texture, None, location, program, texture_bind_points)
        },
    }
}

fn bind_texture_uniform<P, T>(mut ctxt: &mut context::CommandContext,
                              texture: &T, sampler: Option<SamplerBehavior>,
                              location: gl::types::GLint, program: &P,
                              texture_bind_points: &mut Bitsfield)
                              -> Result<(), DrawError> where P: ProgramExt, T: TextureExt
{
    let sampler = if let Some(sampler) = sampler {
        Some(try!(::sampler_object::get_sampler(ctxt, &sampler)))
    } else {
        None
    };

    let sampler = sampler.unwrap_or(0);

    // finding an appropriate texture unit
    let texture_unit =
        ctxt.state.texture_units
            .iter().enumerate()
            .find(|&(unit, content)| {
                content.texture == texture.get_texture_id() && (content.sampler == sampler ||
                                                        !texture_bind_points.is_used(unit as u16))
            })
            .map(|(unit, _)| unit as u16)
            .or_else(|| {
                if ctxt.state.texture_units.len() <
                    ctxt.capabilities.max_combined_texture_image_units as usize
                {
                    Some(ctxt.state.texture_units.len() as u16)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                texture_bind_points.get_unused().expect("Not enough texture units available")
            });
    assert!((texture_unit as gl::types::GLint) <
            ctxt.capabilities.max_combined_texture_image_units);
    texture_bind_points.set_used(texture_unit);

    // updating the program to use the right unit
    program.set_uniform(ctxt, location,
                        &RawUniformValue::SignedInt(texture_unit as gl::types::GLint));

    // updating the state of the texture unit
    if ctxt.state.texture_units.len() <= texture_unit as usize {
        for _ in (ctxt.state.texture_units.len() .. texture_unit as usize + 1) {
            ctxt.state.texture_units.push(Default::default());
        }
    }

    // TODO: do better
    if ctxt.state.texture_units[texture_unit as usize].texture != texture.get_texture_id() ||
       ctxt.state.texture_units[texture_unit as usize].sampler != sampler
    {
        // TODO: what if it's not supported?
        if ctxt.state.active_texture != texture_unit as gl::types::GLenum {
            unsafe { ctxt.gl.ActiveTexture(texture_unit as gl::types::GLenum + gl::TEXTURE0) };
            ctxt.state.active_texture = texture_unit as gl::types::GLenum;
        }

        texture.bind_to_current(ctxt);

        if ctxt.state.texture_units[texture_unit as usize].sampler != sampler {
            assert!(ctxt.version >= &Version(Api::Gl, 3, 3) ||
                    ctxt.extensions.gl_arb_sampler_objects);

            unsafe { ctxt.gl.BindSampler(texture_unit as gl::types::GLenum, sampler); }
            ctxt.state.texture_units[texture_unit as usize].sampler = sampler;
        }
    }

    Ok(())
}
