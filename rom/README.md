# Put your Ratchet & Clank disc image in this folder.

Any name ending in `.iso` works. The setup finds it on its own:

    ./setup.sh

or, with Docker:

    sudo docker compose up

If you have more than one image here, the setup asks which one to use.

The supported discs are **PAL / Europe v1.00** (serial SCES-50916) and
**USA v1.00** (serial SCUS-97199). Other versions (Japan, the Platinum and
Greatest Hits re-releases) are not supported yet.

Your disc image is never committed to git, and it is only ever read.
