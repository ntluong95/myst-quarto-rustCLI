# Project Overview

This research project _template_ explores phenotypic variations in Drosophila melanogaster across different environments, aiming to understand the influence of environmental factors on genetic expression. The study entails collecting samples, observing phenotypic traits, and conducting statistical analyses to determine the significance of observed variations.

## Prerequisites

- Anaconda or Miniconda

## Setting Up the Environment with an `environment.yml` File

Using Conda, you can easily set up your environment and install all necessary dependencies through an `environment.yml` file, which specifies the Python version and packages required for the project.

### Creating the Environment from environment.yml

1. **Install Anaconda or Miniconda:** If not already installed, download and install Anaconda from [the Anaconda website](https://www.anaconda.com/products/individual) or Miniconda from [the Miniconda website](https://docs.conda.io/en/latest/miniconda.html).

2. **Create the Conda Environment:** Navigate to the root directory of the project in your terminal or Anaconda Prompt, where the `environment.yml` file is located. Create the environment by running:

   ```bash
   conda env create -f environment.yml
   ```

   This command creates a new Conda environment as specified in the `environment.yml` file, including installing all required dependencies.

3. **Activate the Environment:** Once the environment is created, you can activate it using:

   ```bash
   conda activate gen_var_study
   ```

## Running the Project

With the Conda environment set up and activated, you are ready to interact with your project in Jupyter or VSCode.
To preview and build the article, we use [MyST](https://mystmd.org) ([install instructions](https://mystmd.org/guide/quickstart)).
MyST can be used to start a local web server to preview your content:

```bash
myst start
```

or to build the PDF, use:

```bash
myst build --pdf
```
